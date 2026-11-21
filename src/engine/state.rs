//! Per engine runtime state: suspensions and statistics, ported from the
//! error handling in `searx/engines/__init__.py` (`ban_time_on_fail`,
//! `max_ban_time_on_fail` and `suspended_times`).

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::FailureKind;
use crate::settings::{CircuitSettings, Search};

/// Consecutive suspensions of one engine climb this ladder and stop at the
/// last entry: 1 h, 6 h, 24 h, 7 days. The first suspension uses the window
/// from `search.suspended_times` and later steps never go below it. A
/// successful request resets the ladder.
const SUSPENSION_BACKOFF_SECONDS: [u64; 4] = [3_600, 21_600, 86_400, 604_800];

/// Counters kept per engine.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EngineStats {
    pub requests: u64,
    pub results: u64,
    pub errors: u64,
    pub timeouts: u64,
    pub http_errors: u64,
    pub total_time: f64,
}

impl EngineStats {
    /// Average response time in seconds.
    pub fn average_time(&self) -> f64 {
        if self.requests == 0 {
            0.0
        } else {
            self.total_time / self.requests as f64
        }
    }
}

/// Circuit breaker phase of one engine.
///
/// Hard blocks (access denied, CAPTCHA, 429) suspend an engine outright
/// through `suspended_times`; the circuit is the softer statistical layer in
/// front of the soft failures (timeouts, HTTP and parse errors). It judges the
/// last `window` outcomes and opens when enough of them failed, so an engine
/// that keeps failing half of the time steps out of the rotation for a while
/// instead of costing every search its deadline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    /// Normal operation.
    #[default]
    Closed,
    /// The engine is skipped until the open period is over.
    Open,
    /// The open period is over: one trial is allowed, its outcome decides.
    HalfOpen,
}

impl CircuitState {
    /// Stable label used by `/stats` and the unresponsive-engine reason.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Open => "open",
            Self::HalfOpen => "half-open",
        }
    }
}

/// What the circuit breaker decides for the engine that is about to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitAdmission {
    /// Run the engine; it is part of the normal rotation.
    Allow,
    /// Run the engine as the single half-open trial. The outcome must be
    /// recorded through [`EngineStates::record_success`] or
    /// [`EngineStates::record_failure`].
    Trial,
    /// Leave the engine out of this search.
    Skip {
        /// `Open`, or `HalfOpen` while another trial is running.
        state: CircuitState,
        /// Time until the circuit may allow the next trial.
        remaining: Duration,
    },
}

/// Rolling outcome window and transition bookkeeping of one engine.
#[derive(Debug, Clone, Default)]
struct Circuit {
    state: CircuitState,
    /// The last outcomes, oldest first; `true` marks a failure.
    outcomes: VecDeque<bool>,
    /// When an open circuit allows its trial.
    open_until: Option<Instant>,
    /// Deadline of the half-open trial that was handed out.
    trial_until: Option<Instant>,
    /// Consecutive re-openings, the position in the open-period ladder.
    open_streak: u32,
}

impl Circuit {
    fn failures(&self) -> usize {
        self.outcomes.iter().filter(|failed| **failed).count()
    }

    fn failure_ratio(&self) -> f64 {
        if self.outcomes.is_empty() {
            0.0
        } else {
            self.failures() as f64 / self.outcomes.len() as f64
        }
    }

    /// Push one outcome into the window, dropping the oldest.
    fn push(&mut self, failed: bool, settings: &CircuitSettings) {
        self.outcomes.push_back(failed);
        while self.outcomes.len() > settings.effective_window() {
            self.outcomes.pop_front();
        }
    }

    /// Whether the current window is bad enough to open the circuit.
    fn overtaxed(&self, settings: &CircuitSettings) -> bool {
        self.outcomes.len() >= settings.effective_minimum_samples()
            && self.failures() > 0
            && self.failure_ratio() >= settings.effective_failure_ratio()
    }

    /// How long the next opening keeps the engine out: the first step is
    /// `open_seconds`, every failed trial doubles it up to
    /// `max_open_seconds`.
    fn open_period(&self, settings: &CircuitSettings) -> Duration {
        let base = settings.effective_open_seconds();
        let ceiling = settings.effective_max_open_seconds();
        let doubled = base.saturating_mul(1u64 << self.open_streak.min(32));
        Duration::from_secs(doubled.min(ceiling))
    }

    fn open(&mut self, settings: &CircuitSettings, now: Instant) {
        self.state = CircuitState::Open;
        self.open_until = Some(now + self.open_period(settings));
        self.trial_until = None;
    }

    fn close(&mut self) {
        self.state = CircuitState::Closed;
        self.open_until = None;
        self.trial_until = None;
        self.open_streak = 0;
        self.outcomes.clear();
    }

    /// Record one engine outcome.
    ///
    /// `counted` is false for a hard block: those suspend through
    /// `suspended_times`, and counting them here would open a circuit for a
    /// reason the suspension already explains. A hard block that ends a
    /// half-open trial still re-opens the circuit, because the trial itself is
    /// what failed.
    fn record(&mut self, failed: bool, counted: bool, settings: &CircuitSettings, now: Instant) {
        match self.state {
            // Requests that were already running when the circuit opened say
            // nothing about the next trial.
            CircuitState::Open => {}
            CircuitState::HalfOpen => {
                self.trial_until = None;
                if failed {
                    self.open_streak = self.open_streak.saturating_add(1);
                    self.open(settings, now);
                } else {
                    self.close();
                }
            }
            CircuitState::Closed if counted => {
                self.push(failed, settings);
                if self.overtaxed(settings) {
                    self.open_streak = 0;
                    self.open(settings, now);
                }
            }
            CircuitState::Closed => {}
        }
    }

    /// Decide whether the engine may start now.
    fn admission(&mut self, settings: &CircuitSettings, now: Instant) -> CircuitAdmission {
        match self.state {
            CircuitState::Closed => CircuitAdmission::Allow,
            CircuitState::Open => match self.open_until {
                Some(until) if until > now => CircuitAdmission::Skip {
                    state: CircuitState::Open,
                    remaining: until - now,
                },
                // The open period is over: allow one trial.
                _ => self.grant_trial(settings, now),
            },
            CircuitState::HalfOpen => match self.trial_until {
                // Another request is already running the trial.
                Some(until) if until > now => CircuitAdmission::Skip {
                    state: CircuitState::HalfOpen,
                    remaining: until - now,
                },
                // The trial never reported back: give the next request one.
                _ => self.grant_trial(settings, now),
            },
        }
    }

    fn grant_trial(&mut self, settings: &CircuitSettings, now: Instant) -> CircuitAdmission {
        self.state = CircuitState::HalfOpen;
        self.trial_until = Some(now + self.open_period(settings));
        CircuitAdmission::Trial
    }

    /// The state to report: an elapsed open period already counts as
    /// half-open, even before a trial was handed out.
    fn report_state(&self, now: Instant) -> CircuitState {
        match self.state {
            CircuitState::Open if self.open_until.is_none_or(|until| until <= now) => {
                CircuitState::HalfOpen
            }
            state => state,
        }
    }

    fn remaining_open(&self, now: Instant) -> Duration {
        match self.state {
            CircuitState::Open => self
                .open_until
                .map(|until| until.saturating_duration_since(now))
                .unwrap_or_default(),
            _ => Duration::ZERO,
        }
    }
}

/// Hard blocks suspend through `suspended_times`; the circuit judges the
/// softer failures that a suspension does not cover well.
fn feeds_circuit(kind: FailureKind) -> bool {
    !matches!(
        kind,
        FailureKind::AccessDenied | FailureKind::Captcha | FailureKind::TooManyRequests
    )
}

/// Record an outcome in the engine's circuit, if the breaker is enabled.
fn observe_circuit(state: &mut EngineState, failed: bool, counted: bool, settings: &Search) {
    if settings.circuit.enabled {
        state
            .circuit
            .record(failed, counted, &settings.circuit, Instant::now());
    }
}

#[derive(Debug, Clone, Default)]
struct EngineState {
    consecutive_failures: u64,
    /// Successful answers that carried no results. An engine that keeps
    /// answering nothing is costing every search its timeout for free.
    empty_streak: u64,
    /// When the engine may rejoin the rotation.
    quiet_until: Option<Instant>,
    suspended_until: Option<Instant>,
    suspension_reason: Option<String>,
    /// Number of consecutive hard-block suspensions, the backoff ladder
    /// position. Reset by a successful request.
    suspension_streak: u64,
    circuit: Circuit,
    stats: EngineStats,
}

/// A read-only view of one engine's state, used by `/stats`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineSnapshot {
    pub engine: String,
    pub stats: EngineStats,
    pub consecutive_failures: u64,
    pub suspended: bool,
    pub suspended_for: f64,
    pub suspension_reason: Option<String>,
    /// Engine is resting due to repeated empty results.
    pub resting: bool,
    /// Remaining rest time in seconds.
    pub resting_for: f64,
    /// Circuit breaker phase (`closed`, `open` or `half_open`).
    pub circuit: CircuitState,
    /// Seconds until a half-open trial is allowed; 0 unless the circuit is open.
    pub circuit_open_for: f64,
    /// Failure share of the rolling window.
    pub circuit_failure_ratio: f64,
    /// Outcomes currently in the rolling window.
    pub circuit_samples: u64,
}

impl EngineSnapshot {
    /// The state of a configured engine that has not run yet.
    ///
    /// `EngineStates::snapshot` only lists engines that already have state, so
    /// a process that has just started reported nothing at all — which is when
    /// an operator is most likely to look.
    pub fn idle(engine: &str) -> Self {
        Self {
            engine: engine.to_string(),
            stats: EngineStats::default(),
            consecutive_failures: 0,
            suspended: false,
            suspended_for: 0.0,
            suspension_reason: None,
            resting: false,
            resting_for: 0.0,
            circuit: CircuitState::Closed,
            circuit_open_for: 0.0,
            circuit_failure_ratio: 0.0,
            circuit_samples: 0,
        }
    }

    /// The state of a configured engine that could not be initialised.
    ///
    /// The engine cannot run at all, so it reports as suspended with the
    /// build failure as its reason; `suspended_for` stays 0 because the
    /// suspension is not time-bound.
    pub fn unavailable(engine: &str, reason: &str) -> Self {
        Self {
            suspended: true,
            suspension_reason: Some(format!("could not be initialised: {reason}")),
            ..Self::idle(engine)
        }
    }

    /// Circuit phase as a human-readable label (see [`CircuitState::as_str`]).
    pub fn circuit_label(&self) -> &'static str {
        self.circuit.as_str()
    }

    /// Failure share of the rolling window as a percentage, rounded to one
    /// decimal for display.
    pub fn circuit_failure_percent(&self) -> f64 {
        (self.circuit_failure_ratio * 1_000.0).round() / 10.0
    }

    /// Remaining open time rounded up to whole seconds; 0 unless the circuit
    /// is open.
    pub fn circuit_open_seconds(&self) -> u64 {
        self.circuit_open_for.ceil() as u64
    }
}

/// Thread safe map of engine states.
#[derive(Debug, Default)]
pub struct EngineStates {
    states: Mutex<HashMap<String, EngineState>>,
}

impl EngineStates {
    pub fn new() -> Self {
        Self::default()
    }

    fn with_state<T>(&self, engine: &str, action: impl FnOnce(&mut EngineState) -> T) -> T {
        // A panic elsewhere must not turn every later search into a panic:
        // recover the guard like the response cache does.
        let mut states = self
            .states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = states.entry(engine.to_string()).or_default();
        action(state)
    }

    /// Remaining suspension time, if the engine is suspended.
    pub fn suspension(&self, engine: &str) -> Option<(Duration, Option<String>)> {
        self.with_state(engine, |state| {
            // An engine that keeps answering nothing is rested, not failed.
            if let Some(until) = state.quiet_until {
                let now = Instant::now();
                if until > now {
                    return Some((until - now, Some("resting".to_string())));
                }
                state.quiet_until = None;
            }
            state.suspended_until.and_then(|until| {
                let now = Instant::now();
                if until > now {
                    Some((until - now, state.suspension_reason.clone()))
                } else {
                    state.suspended_until = None;
                    state.suspension_reason = None;
                    None
                }
            })
        })
    }

    /// Consult the circuit breaker before starting an engine.
    ///
    /// A [`CircuitAdmission::Trial`] consumes the single half-open trial, so
    /// the caller has to record the outcome afterwards even when it decides
    /// not to run the engine.
    pub fn circuit_admission(&self, engine: &str, settings: &Search) -> CircuitAdmission {
        if !settings.circuit.enabled {
            return CircuitAdmission::Allow;
        }
        self.with_state(engine, |state| {
            state.circuit.admission(&settings.circuit, Instant::now())
        })
    }

    /// Record a successful request.
    pub fn record_success(
        &self,
        engine: &str,
        results: usize,
        elapsed: Duration,
        settings: &Search,
    ) {
        self.with_state(engine, |state| {
            observe_circuit(state, false, true, settings);
            state.consecutive_failures = 0;
            state.suspension_streak = 0;
            if results == 0 {
                state.empty_streak += 1;
                if settings.empty_result_limit > 0
                    && state.empty_streak >= settings.empty_result_limit
                {
                    state.quiet_until = Some(
                        Instant::now() + Duration::from_secs(settings.empty_result_cooldown.max(1)),
                    );
                    state.empty_streak = 0;
                }
            } else {
                state.empty_streak = 0;
                state.quiet_until = None;
            }
            state.suspended_until = None;
            state.suspension_reason = None;
            state.stats.requests += 1;
            state.stats.results += results as u64;
            state.stats.total_time += elapsed.as_secs_f64();
        });
    }

    /// Record a failure and return the suspension time in seconds (0 = none).
    pub fn record_failure(&self, engine: &str, kind: FailureKind, settings: &Search) -> u64 {
        self.with_state(engine, |state| {
            observe_circuit(state, true, feeds_circuit(kind), settings);
            state.stats.errors += 1;
            if kind == FailureKind::Timeout {
                state.stats.timeouts += 1;
            }
            if kind == FailureKind::Http {
                state.stats.http_errors += 1;
            }
            state.consecutive_failures += 1;

            let suspension = match kind.suspension_key() {
                Some(_) if !kind.starts_suspension(state.consecutive_failures) => 0,
                Some(key) => {
                    let configured = settings.suspended_times.get(key).copied().unwrap_or(3_600);
                    // The configured window overrides the first step and stays
                    // the floor afterwards, so a previously long window never
                    // shrinks on the second repeat.
                    let seconds = if state.suspension_streak == 0 {
                        configured
                    } else {
                        SUSPENSION_BACKOFF_SECONDS[(state.suspension_streak as usize)
                            .min(SUSPENSION_BACKOFF_SECONDS.len() - 1)]
                        .max(configured)
                    };
                    if seconds > 0 {
                        state.suspension_streak = state.suspension_streak.saturating_add(1);
                    }
                    seconds
                }
                None => {
                    // Repeated soft failures degrade into a short suspension,
                    // exactly like upstream's ban_time_on_fail.
                    if state.consecutive_failures >= 2 {
                        (state.consecutive_failures * settings.ban_time_on_fail)
                            .min(settings.max_ban_time_on_fail)
                    } else {
                        0
                    }
                }
            };
            if suspension > 0 {
                state.suspended_until = Some(Instant::now() + Duration::from_secs(suspension));
                state.suspension_reason = Some(kind.error_type().to_string());
            }
            suspension
        })
    }

    /// Reset the counters (used by tests and by `/stats` reset).
    pub fn reset(&self) {
        self.states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    /// Snapshot every configured engine, including the ones that never ran.
    ///
    /// Operators read `/stats` and `/metrics` right after a restart, when
    /// nothing has been used yet; reporting an empty table there hides the
    /// engines the process is about to try. Engines that failed to
    /// initialise are listed as suspended with their build failure, so a
    /// broken configuration is visible in the same place.
    pub fn snapshot_configured(
        &self,
        configured: &[String],
        failed: &[(String, String)],
    ) -> Vec<EngineSnapshot> {
        let mut snapshots = self.snapshot();
        for name in configured {
            if !snapshots.iter().any(|snapshot| &snapshot.engine == name) {
                snapshots.push(EngineSnapshot::idle(name));
            }
        }
        for (name, reason) in failed {
            if !snapshots.iter().any(|snapshot| &snapshot.engine == name) {
                snapshots.push(EngineSnapshot::unavailable(name, reason));
            }
        }
        snapshots.sort_by(|a, b| a.engine.cmp(&b.engine));
        snapshots
    }

    /// Snapshot of every engine that has been used.
    pub fn snapshot(&self) -> Vec<EngineSnapshot> {
        let now = Instant::now();
        let states = self
            .states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut snapshots: Vec<EngineSnapshot> = states
            .iter()
            .map(|(engine, state)| {
                let (suspended, suspended_for, suspension_reason) = match state.suspended_until {
                    Some(until) if until > now => (
                        true,
                        (until - now).as_secs_f64(),
                        state.suspension_reason.clone(),
                    ),
                    _ => (false, 0.0, None),
                };
                let (resting, resting_for) = match state.quiet_until {
                    Some(until) if until > now => (true, (until - now).as_secs_f64()),
                    _ => (false, 0.0),
                };
                EngineSnapshot {
                    engine: engine.clone(),
                    stats: state.stats.clone(),
                    consecutive_failures: state.consecutive_failures,
                    suspended,
                    suspended_for,
                    suspension_reason,
                    resting,
                    resting_for,
                    circuit: state.circuit.report_state(now),
                    circuit_open_for: state.circuit.remaining_open(now).as_secs_f64(),
                    circuit_failure_ratio: state.circuit.failure_ratio(),
                    circuit_samples: state.circuit.outcomes.len() as u64,
                }
            })
            .collect();
        snapshots.sort_by(|a, b| a.engine.cmp(&b.engine));
        snapshots
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_denied_suspends_for_an_hour() {
        let states = EngineStates::new();
        let settings = Search::default();
        let seconds = states.record_failure("duckduckgo", FailureKind::AccessDenied, &settings);
        assert_eq!(seconds, 3_600);
        assert!(states.suspension("duckduckgo").is_some());
    }

    #[test]
    fn timeouts_do_not_rest_an_engine_by_default() {
        let states = EngineStates::new();
        let settings = Search::default();

        // The bounded parallel wait cuts a slow engine short, so resting it
        // would only remove results. Repeated timeouts stay unsuspended.
        for _ in 0..3 {
            let seconds = states.record_failure("slow", FailureKind::Timeout, &settings);
            assert_eq!(seconds, 0);
            assert!(states.suspension("slow").is_none());
        }
    }

    #[test]
    fn a_configured_timeout_window_rests_the_engine_after_two_failures() {
        let states = EngineStates::new();
        let mut settings = Search::default();
        settings
            .suspended_times
            .insert("SearxEngineTimeout".into(), 60);

        // One congested answer says nothing about the provider.
        let first = states.record_failure("slow", FailureKind::Timeout, &settings);
        assert_eq!(first, 0);
        assert!(states.suspension("slow").is_none());

        let second = states.record_failure("slow", FailureKind::Timeout, &settings);
        assert_eq!(second, 60);
        assert!(states.suspension("slow").is_some());
    }

    #[test]
    fn a_success_between_timeouts_clears_the_streak() {
        let states = EngineStates::new();
        let settings = Search::default();
        states.record_failure("slow", FailureKind::Timeout, &settings);
        states.record_success("slow", 3, Duration::from_millis(90), &settings);

        // The next timeout is the first of a new streak again.
        let again = states.record_failure("slow", FailureKind::Timeout, &settings);
        assert_eq!(again, 0);
        assert!(states.suspension("slow").is_none());
    }

    #[test]
    fn repeated_captchas_back_off_to_seven_days() {
        let states = EngineStates::new();
        let settings = Search::default();
        let ladder: Vec<u64> = (0..6)
            .map(|_| states.record_failure("eng", FailureKind::Captcha, &settings))
            .collect();

        // 1 h -> 6 h -> 24 h -> 7 days, then the cap repeats.
        assert_eq!(ladder, [3_600, 21_600, 86_400, 604_800, 604_800, 604_800]);
    }

    #[test]
    fn a_configured_window_overrides_the_first_step_only() {
        let states = EngineStates::new();
        let mut settings = Search::default();
        settings
            .suspended_times
            .insert("SearxEngineCaptcha".into(), 7_200);

        let first = states.record_failure("eng", FailureKind::Captcha, &settings);
        let second = states.record_failure("eng", FailureKind::Captcha, &settings);
        assert_eq!(first, 7_200, "the configured value is the first step");
        assert_eq!(second, 21_600, "repeats use the shared ladder");

        // A long configured window (the historical one-day default) never
        // shrinks to the 6 h ladder step on the second repeat.
        let mut settings = Search::default();
        settings
            .suspended_times
            .insert("SearxEngineCaptcha".into(), 86_400);
        let states = EngineStates::new();
        let ladder: Vec<u64> = (0..5)
            .map(|_| states.record_failure("eng", FailureKind::Captcha, &settings))
            .collect();
        assert_eq!(ladder, [86_400, 86_400, 86_400, 604_800, 604_800]);
    }

    #[test]
    fn a_success_resets_the_backoff_ladder() {
        let states = EngineStates::new();
        let settings = Search::default();
        states.record_failure("eng", FailureKind::Captcha, &settings);
        states.record_failure("eng", FailureKind::Captcha, &settings);
        states.record_success("eng", 1, Duration::from_millis(50), &settings);

        let again = states.record_failure("eng", FailureKind::Captcha, &settings);
        assert_eq!(again, 3_600, "a success starts the ladder over");
    }

    #[test]
    fn too_many_requests_keeps_its_short_first_window_and_backs_off() {
        let states = EngineStates::new();
        let settings = Search::default();

        let first = states.record_failure("eng", FailureKind::TooManyRequests, &settings);
        let second = states.record_failure("eng", FailureKind::TooManyRequests, &settings);
        assert_eq!(first, 300);
        assert!(
            second > first,
            "a repeated 429 must back off, got {second} after {first}"
        );
        assert_eq!(second, 21_600);

        // A soft failure does not advance the hard-block ladder.
        let fresh = EngineStates::new();
        fresh.record_failure("eng", FailureKind::Other, &settings);
        fresh.record_failure("eng", FailureKind::Other, &settings);
        let captcha = fresh.record_failure("eng", FailureKind::Captcha, &settings);
        assert_eq!(captcha, 3_600);
    }

    #[test]
    fn two_soft_failures_trigger_a_short_suspension() {
        let states = EngineStates::new();
        let settings = Search::default();
        assert_eq!(states.record_failure("x", FailureKind::Other, &settings), 0);
        let second = states.record_failure("x", FailureKind::Other, &settings);
        assert_eq!(second, 10);
        assert!(states.suspension("x").is_some());
    }

    #[test]
    fn success_clears_the_state() {
        let states = EngineStates::new();
        let settings = Search::default();
        states.record_failure("x", FailureKind::TooManyRequests, &settings);
        states.record_success("x", 3, Duration::from_millis(100), &Search::default());
        assert!(states.suspension("x").is_none());
        let snapshot = states.snapshot();
        assert_eq!(snapshot[0].stats.results, 3);
        assert_eq!(snapshot[0].consecutive_failures, 0);
    }

    #[test]
    fn empty_results_below_limit_do_not_rest() {
        let states = EngineStates::new();
        let settings = Search {
            empty_result_limit: 3,
            empty_result_cooldown: 60,
            ..Default::default()
        };

        // N-1 empty results: no resting yet
        for _ in 0..2 {
            states.record_success("eng", 0, Duration::from_millis(50), &settings);
        }
        let snapshot = states.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert!(!snapshot[0].resting);
        assert_eq!(snapshot[0].resting_for, 0.0);
    }

    #[test]
    fn empty_results_at_limit_trigger_resting() {
        let states = EngineStates::new();
        let settings = Search {
            empty_result_limit: 3,
            empty_result_cooldown: 60,
            ..Default::default()
        };

        // Exactly N empty results triggers rest
        for _ in 0..3 {
            states.record_success("eng", 0, Duration::from_millis(50), &settings);
        }
        let snapshot = states.snapshot();
        assert!(snapshot[0].resting);
        assert!(snapshot[0].resting_for > 0.0);

        // suspension() should return the resting status
        let sus = states.suspension("eng");
        assert!(sus.is_some());
        let (_, reason) = sus.unwrap();
        assert_eq!(reason.as_deref(), Some("resting"));
    }

    #[test]
    fn cooldown_expiry_clears_resting() {
        let states = EngineStates::new();
        let settings = Search {
            empty_result_limit: 2,
            empty_result_cooldown: 60,
            ..Default::default()
        };

        // Trigger resting
        states.record_success("eng", 0, Duration::from_millis(50), &settings);
        states.record_success("eng", 0, Duration::from_millis(50), &settings);
        assert!(states.suspension("eng").is_some());

        // Simulate cooldown expiry by setting quiet_until to the past
        states.with_state("eng", |state| {
            state.quiet_until = Some(Instant::now() - Duration::from_secs(1));
        });

        // snapshot should return resting=false before suspension() cleans up
        let snapshot = states.snapshot();
        assert!(!snapshot[0].resting);
        assert_eq!(snapshot[0].resting_for, 0.0);

        // suspension() also returns None after expiry
        assert!(states.suspension("eng").is_none());
    }

    #[test]
    fn actual_results_clear_resting_and_reset_streak() {
        let states = EngineStates::new();
        let settings = Search {
            empty_result_limit: 2,
            empty_result_cooldown: 60,
            ..Default::default()
        };

        // Trigger resting
        states.record_success("eng", 0, Duration::from_millis(50), &settings);
        states.record_success("eng", 0, Duration::from_millis(50), &settings);
        assert!(states.suspension("eng").is_some());

        // Return actual results - should clear resting immediately
        states.record_success("eng", 5, Duration::from_millis(100), &settings);

        assert!(states.suspension("eng").is_none());
        let snapshot = states.snapshot();
        assert!(!snapshot[0].resting);
        assert_eq!(snapshot[0].resting_for, 0.0);

        // The empty streak counter should be reset, so N-1 more empties don't rest
        states.record_success("eng", 0, Duration::from_millis(50), &settings);
        assert!(states.suspension("eng").is_none());
    }

    #[test]
    fn limit_zero_disables_resting() {
        let states = EngineStates::new();
        let settings = Search {
            empty_result_limit: 0, // disabled
            empty_result_cooldown: 60,
            ..Default::default()
        };

        // Many empty results should not trigger resting
        for _ in 0..100 {
            states.record_success("eng", 0, Duration::from_millis(50), &settings);
        }
        assert!(states.suspension("eng").is_none());
        let snapshot = states.snapshot();
        assert!(!snapshot[0].resting);
    }

    #[test]
    fn resting_distinct_from_failure_suspension() {
        let states = EngineStates::new();
        let settings = Search {
            empty_result_limit: 2,
            empty_result_cooldown: 60,
            ..Default::default()
        };

        // Trigger failure suspension
        states.record_failure("a", FailureKind::AccessDenied, &settings);
        let snap_a = states
            .snapshot()
            .into_iter()
            .find(|s| s.engine == "a")
            .unwrap();
        assert!(snap_a.suspended);
        assert!(!snap_a.resting);
        assert!(snap_a.suspension_reason.is_some());

        // Trigger resting
        states.record_success("b", 0, Duration::from_millis(50), &settings);
        states.record_success("b", 0, Duration::from_millis(50), &settings);
        let snap_b = states
            .snapshot()
            .into_iter()
            .find(|s| s.engine == "b")
            .unwrap();
        assert!(!snap_b.suspended);
        assert!(snap_b.resting);
        assert!(snap_b.suspension_reason.is_none());

        // Both appear in suspension() but with different reasons
        let (_, reason_a) = states.suspension("a").unwrap();
        let (_, reason_b) = states.suspension("b").unwrap();
        assert_ne!(reason_a, reason_b);
        assert_eq!(reason_b.as_deref(), Some("resting"));
    }

    /// Settings with a circuit that opens on the tested window.
    fn circuit_search(circuit: CircuitSettings) -> Search {
        Search {
            circuit,
            ..Default::default()
        }
    }

    fn circuit(window: usize, minimum_samples: usize, ratio: f64) -> CircuitSettings {
        CircuitSettings {
            window,
            minimum_samples,
            failure_ratio: ratio,
            open_seconds: 30,
            max_open_seconds: 600,
            ..Default::default()
        }
    }

    /// Move an open circuit past its open period, the way wall-clock time
    /// would after `open_seconds`.
    fn expire_open_period(states: &EngineStates, engine: &str) {
        states.with_state(engine, |state| {
            state.circuit.open_until = Some(Instant::now() - Duration::from_secs(1));
            state.circuit.trial_until = None;
        });
    }

    fn circuit_of(states: &EngineStates, engine: &str) -> EngineSnapshot {
        states
            .snapshot()
            .into_iter()
            .find(|snapshot| snapshot.engine == engine)
            .expect("the engine is tracked")
    }

    #[test]
    fn circuit_opens_when_the_window_reaches_the_failure_ratio() {
        let states = EngineStates::new();
        let settings = circuit_search(circuit(4, 4, 0.5));

        // Three outcomes say nothing yet, even with a failure in every one.
        states.record_failure("eng", FailureKind::Timeout, &settings);
        states.record_success("eng", 1, Duration::from_millis(20), &settings);
        states.record_failure("eng", FailureKind::Timeout, &settings);
        assert_eq!(
            states.circuit_admission("eng", &settings),
            CircuitAdmission::Allow,
            "the minimum sample count is not reached"
        );

        // The fourth outcome completes the window at exactly 50%: it opens.
        states.record_success("eng", 1, Duration::from_millis(20), &settings);
        match states.circuit_admission("eng", &settings) {
            CircuitAdmission::Skip { state, remaining } => {
                assert_eq!(state, CircuitState::Open);
                assert!(remaining > Duration::ZERO);
            }
            other => panic!("the circuit must be open at the threshold, got {other:?}"),
        }

        let snapshot = circuit_of(&states, "eng");
        assert_eq!(snapshot.circuit, CircuitState::Open);
        assert!(snapshot.circuit_open_for > 0.0);
        assert_eq!(snapshot.circuit_samples, 4);
        assert!((snapshot.circuit_failure_ratio - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_window_below_the_failure_ratio_stays_closed() {
        let states = EngineStates::new();
        let settings = circuit_search(circuit(4, 4, 0.5));
        states.record_failure("eng", FailureKind::Timeout, &settings);
        for _ in 0..3 {
            states.record_success("eng", 1, Duration::from_millis(20), &settings);
        }
        assert_eq!(
            states.circuit_admission("eng", &settings),
            CircuitAdmission::Allow,
            "one failure in four outcomes is below the threshold"
        );
        assert_eq!(circuit_of(&states, "eng").circuit, CircuitState::Closed);
    }

    #[test]
    fn an_open_circuit_allows_one_trial_after_the_open_period() {
        let states = EngineStates::new();
        let settings = circuit_search(circuit(2, 2, 1.0));
        states.record_failure("eng", FailureKind::Timeout, &settings);
        states.record_failure("eng", FailureKind::Timeout, &settings);

        assert!(matches!(
            states.circuit_admission("eng", &settings),
            CircuitAdmission::Skip {
                state: CircuitState::Open,
                ..
            }
        ));

        expire_open_period(&states, "eng");
        assert_eq!(
            states.circuit_admission("eng", &settings),
            CircuitAdmission::Trial,
            "the open period is over: one trial is allowed"
        );
        assert!(
            matches!(
                states.circuit_admission("eng", &settings),
                CircuitAdmission::Skip {
                    state: CircuitState::HalfOpen,
                    ..
                }
            ),
            "a running trial is not doubled by a concurrent request"
        );
        assert_eq!(circuit_of(&states, "eng").circuit, CircuitState::HalfOpen);
    }

    #[test]
    fn a_successful_trial_closes_the_circuit() {
        let states = EngineStates::new();
        let settings = circuit_search(circuit(2, 2, 1.0));
        states.record_failure("eng", FailureKind::Timeout, &settings);
        states.record_failure("eng", FailureKind::Timeout, &settings);
        expire_open_period(&states, "eng");
        assert_eq!(
            states.circuit_admission("eng", &settings),
            CircuitAdmission::Trial
        );

        states.record_success("eng", 2, Duration::from_millis(30), &settings);

        assert_eq!(
            states.circuit_admission("eng", &settings),
            CircuitAdmission::Allow,
            "a successful trial returns the engine to the rotation"
        );
        let snapshot = circuit_of(&states, "eng");
        assert_eq!(snapshot.circuit, CircuitState::Closed);
        assert_eq!(snapshot.circuit_open_for, 0.0);
        assert_eq!(
            snapshot.circuit_samples, 0,
            "the window starts over after a close"
        );
    }

    #[test]
    fn a_failed_trial_reopens_with_a_longer_bounded_period() {
        let states = EngineStates::new();
        let settings = circuit_search(circuit(2, 2, 1.0));
        states.record_failure("eng", FailureKind::Timeout, &settings);
        states.record_failure("eng", FailureKind::Timeout, &settings);
        let first = circuit_of(&states, "eng").circuit_open_for;
        assert!(first > 0.0 && first <= 30.0, "first period: {first}");

        expire_open_period(&states, "eng");
        assert_eq!(
            states.circuit_admission("eng", &settings),
            CircuitAdmission::Trial
        );
        states.record_failure("eng", FailureKind::Timeout, &settings);
        let second = circuit_of(&states, "eng").circuit_open_for;
        assert_eq!(circuit_of(&states, "eng").circuit, CircuitState::Open);
        assert!(
            second > 55.0 && second <= 60.0,
            "a failed trial doubles the period: {second}"
        );

        // The ladder is bounded: with a 45 second ceiling the next re-open
        // stops there instead of doubling to 120.
        let states = EngineStates::new();
        let settings = circuit_search(CircuitSettings {
            max_open_seconds: 45,
            ..circuit(2, 2, 1.0)
        });
        states.record_failure("eng", FailureKind::Timeout, &settings);
        states.record_failure("eng", FailureKind::Timeout, &settings);
        expire_open_period(&states, "eng");
        assert_eq!(
            states.circuit_admission("eng", &settings),
            CircuitAdmission::Trial
        );
        states.record_failure("eng", FailureKind::Timeout, &settings);
        let capped = circuit_of(&states, "eng").circuit_open_for;
        assert!(
            capped > 40.0 && capped <= 45.0,
            "the period is capped at max_open_seconds: {capped}"
        );
    }

    #[test]
    fn a_disabled_circuit_never_opens() {
        let states = EngineStates::new();
        let settings = circuit_search(CircuitSettings {
            enabled: false,
            ..circuit(2, 2, 1.0)
        });
        for _ in 0..10 {
            states.record_failure("eng", FailureKind::Timeout, &settings);
            assert_eq!(
                states.circuit_admission("eng", &settings),
                CircuitAdmission::Allow
            );
        }
        let snapshot = circuit_of(&states, "eng");
        assert_eq!(snapshot.circuit, CircuitState::Closed);
        assert_eq!(
            snapshot.circuit_samples, 0,
            "a disabled circuit keeps no window"
        );
    }

    #[test]
    fn hard_blocks_do_not_feed_the_circuit() {
        let states = EngineStates::new();
        let settings = circuit_search(circuit(2, 2, 1.0));

        // A hard block owns its suspension; the statistical layer stays out
        // of the way.
        states.record_failure("eng", FailureKind::AccessDenied, &settings);
        states.record_failure("eng", FailureKind::Captcha, &settings);
        states.record_failure("eng", FailureKind::TooManyRequests, &settings);

        assert_eq!(
            states.circuit_admission("eng", &settings),
            CircuitAdmission::Allow,
            "hard blocks are the suspension ladder's business"
        );
        assert_eq!(circuit_of(&states, "eng").circuit, CircuitState::Closed);
    }

    #[test]
    fn configured_engines_are_reported_before_their_first_request() {
        let states = EngineStates::new();
        let names = vec!["zeta".to_string(), "alpha".to_string()];
        let failed = vec![("kaput".to_string(), "missing api key".to_string())];
        let idle = states.snapshot_configured(&names, &failed);
        assert_eq!(
            idle.iter().map(|s| s.engine.as_str()).collect::<Vec<_>>(),
            ["alpha", "kaput", "zeta"]
        );
        assert!(idle.iter().all(|s| s.circuit == CircuitState::Closed));
        assert!(
            idle.iter()
                .all(|s| s.stats.requests == 0 && s.circuit_samples == 0)
        );
        // An engine that failed to initialise shows as suspended with its
        // reason instead of vanishing from the report entirely.
        assert!(idle[1].suspended);
        assert_eq!(
            idle[1].suspension_reason.as_deref(),
            Some("could not be initialised: missing api key")
        );

        // An engine that has run keeps its real counters, and the list stays
        // sorted and free of duplicates.
        let settings = crate::settings::Search::default();
        states.record_success("zeta", 3, Duration::from_millis(10), &settings);
        let mixed = states.snapshot_configured(&names, &failed);
        assert_eq!(mixed.len(), 3);
        assert_eq!(mixed[2].engine, "zeta");
        assert_eq!(mixed[2].stats.requests, 1);
    }

    #[test]
    fn snapshot_clears_expired_suspension_reason() {
        let states = EngineStates::new();
        let settings = Search::default();

        // Create a suspension
        states.record_failure("eng", FailureKind::AccessDenied, &settings);
        let snap = states
            .snapshot()
            .into_iter()
            .find(|s| s.engine == "eng")
            .unwrap();
        assert!(snap.suspended);
        assert!(snap.suspension_reason.is_some());

        // Simulate expiry by setting suspended_until to the past
        states.with_state("eng", |state| {
            state.suspended_until = Some(Instant::now() - Duration::from_secs(1));
            state.suspension_reason = Some("access_denied".to_string());
        });

        // Snapshot should clear the reason since suspension is expired
        let snap = states
            .snapshot()
            .into_iter()
            .find(|s| s.engine == "eng")
            .unwrap();
        assert!(!snap.suspended);
        assert!(snap.suspension_reason.is_none());
    }
}
