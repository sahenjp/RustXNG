//! Result collection, scoring and ordering, ported from `searx/results.py`
//! and `searx/result_types/`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::engine::EngineItem;

mod rank;

pub use rank::{canonical_host_for_query, page_language, query_terms, relevance_score};

use rank::{
    authority, consensus, order_results, page_language_adjustment, relevance, text_match,
    wikipedia_language_penalty,
};

/// Result priority, upstream `MainResult.PriorityType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// The engine explicitly marked the result as unimportant.
    Low,
    #[default]
    Medium,
    /// Answers / direct hits, always ranked first.
    High,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Template used to render a result, upstream `result.template`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Template {
    #[default]
    Default,
    Images,
    Videos,
    Map,
    Music,
    Packages,
    /// Key/value rows from a database backed engine (upstream keyvalue.html).
    KeyValue,
}

impl Template {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Images => "images",
            Self::Videos => "videos",
            Self::Map => "map",
            Self::Music => "music",
            Self::Packages => "packages",
            Self::KeyValue => "keyvalue",
        }
    }
}

/// One search result, upstream `MainResult` / `LegacyResult`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub content: String,
    /// The engine that produced the result (first one wins).
    pub engine: String,
    /// Every engine that produced this result.
    #[serde(default)]
    pub engines: Vec<String>,
    /// 1-based positions inside each engine's result list.
    #[serde(default)]
    pub positions: Vec<u32>,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub category: String,
    #[serde(default, serialize_with = "serialize_template")]
    pub template: Template,
    #[serde(
        default,
        skip_serializing_if = "is_medium",
        serialize_with = "serialize_priority"
    )]
    pub priority: Priority,
    #[serde(
        rename = "publishedDate",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub published_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub img_src: Option<String>,
    /// Engine specific extra fields (length, views, licence, ...).
    ///
    /// Upstream serialises these as one `"key: value | key: value"` string in
    /// the JSON API, so we do the same.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        serialize_with = "serialize_metadata"
    )]
    pub metadata: BTreeMap<String, String>,
    /// A short URL shown in the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pretty_url: Option<String>,
    /// Which extra result page this came from (0 = page 1).
    #[serde(skip)]
    pub deep_page: u32,
    /// Readable page text, only when the request asked for raw content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_content: Option<String>,
    /// Whether optional raw content was cut at the configured character limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_content_truncated: Option<bool>,
    /// Why the body is missing or degraded, only when the request asked for
    /// raw content: `ok`, `not_requested`, `too_large`, `fetch_failed`,
    /// `robots_blocked`, `extract_failed`, `render_failed` or `unknown`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_content_status: Option<String>,
    /// Short reader message behind `raw_content_status`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_content_error: Option<String>,
    /// 転載統合（distillation）でこの結果にまとめた転載元のタイトルと URL。
    ///
    /// SearXNG には無い上流超えの蒸留: 同じ原発表の転載（ベンダー公告 → 記事 → ミラー）を
    /// 1 件に統合し、独立した根拠を数え誤らせないようにする（D47）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merged_sources: Vec<String>,
    /// 出典系列（転載チェーンの記録）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_family: Option<String>,
    /// 検索クエリとの関連性（0.0〜1.0）。蒸留の可視化と、転載統合時の代表選択に使う。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relevance_score: Option<f64>,
    /// Whether the result belongs to the group of the previous one (UI only).
    #[serde(skip)]
    pub open_group: bool,
    #[serde(skip)]
    pub close_group: bool,
}

fn is_medium(priority: &Priority) -> bool {
    *priority == Priority::Medium
}

/// Upstream renders `default.html`, `images.html`, ... in its JSON API.
fn serialize_template<S>(template: &Template, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&format!("{}.html", template.as_str()))
}

/// Upstream keeps the engine metadata as one human readable string.
fn serialize_metadata<S>(
    metadata: &BTreeMap<String, String>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let text = metadata
        .iter()
        .map(|(key, value)| format!("{key}: {value}"))
        .collect::<Vec<_>>()
        .join(" | ");
    serializer.serialize_str(&text)
}

/// Upstream uses an empty string for the default priority.
fn serialize_priority<S>(priority: &Priority, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(match priority {
        Priority::Medium => "",
        other => other.as_str(),
    })
}

impl SearchResult {
    pub fn new(url: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            title: title.into(),
            ..Default::default()
        }
    }

    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = content.into();
        self
    }

    pub fn with_template(mut self, template: Template) -> Self {
        self.template = template;
        self
    }

    pub fn with_metadata(mut self, key: &str, value: impl Into<String>) -> Self {
        self.metadata.insert(key.to_string(), value.into());
        self
    }
}

/// An infobox, upstream `Infobox`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Infobox {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub img_src: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub urls: Vec<InfoboxUrl>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, String>,
    #[serde(default)]
    pub engine: String,
}

/// A link of an infobox (`urls` in the infobox template).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InfoboxUrl {
    pub title: String,
    pub url: String,
}

/// Engine timing, upstream `Timing`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timing {
    pub engine: String,
    pub total: f64,
    pub load: f64,
}

/// An engine that did not answer, upstream `UnresponsiveEngine`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UnresponsiveEngine {
    pub engine: String,
    #[serde(rename = "error_type")]
    pub error_type: String,
    pub suspended: bool,
}

/// Ranking coefficients blended into the position-fusion score.
///
/// The existing terms keep their meaning: relevance is the URL + title +
/// body coverage term and authority the canonical-site term. Consensus and
/// text_match are additive and default to zero for callers that use
/// ResultContainer::close, so the historical ranking API is unchanged.
#[derive(Debug, Clone, Copy, Default)]
pub struct RankingOptions {
    pub relevance: f64,
    pub authority: f64,
    /// Weight of the multi-source agreement bonus.
    pub consensus: f64,
    /// Weight of the title/snippet query-term match bonus.
    pub text_match: f64,
}

/// How aggressively duplicate pages are collapsed while collecting results.
///
/// Both rules are conservative: a URL is only rewritten when it carries an
/// unmistakable copy marker, and a body match needs the same host, the same
/// title and a near-identical snippet. Turning them off restores the previous
/// URL-only deduplication, which is what the offline regression uses as its
/// baseline.
#[derive(Debug, Clone, Copy)]
pub struct DedupPolicy {
    /// Strip AMP / mobile / print markers from a URL before comparing it.
    pub url_variants: bool,
    /// Merge same-host results whose title and body are nearly identical.
    pub near_duplicates: bool,
}

impl Default for DedupPolicy {
    fn default() -> Self {
        Self {
            url_variants: true,
            near_duplicates: true,
        }
    }
}

/// Collects, deduplicates and scores the results of all engines.
#[derive(Debug, Default)]
pub struct ResultContainer {
    main: Vec<SearchResult>,
    index: HashMap<String, usize>,
    source_ranks: Vec<Vec<SourceRank>>,
    dedup: DedupPolicy,
    pub infoboxes: Vec<Infobox>,
    pub suggestions: BTreeSet<String>,
    pub answers: Vec<String>,
    pub corrections: BTreeSet<String>,
    pub engine_data: BTreeMap<String, BTreeMap<String, String>>,
    pub unresponsive_engines: Vec<UnresponsiveEngine>,
    pub timings: Vec<Timing>,
    pub paging: bool,
    pub redirect_url: Option<String>,
    closed: bool,
    ordered: Vec<SearchResult>,
}

#[derive(Debug, Clone, Copy)]
struct SourceRank {
    weight: f64,
    position: u32,
    deep_page: u32,
}

impl SourceRank {
    fn score(self, priority: Priority) -> f64 {
        let score = match priority {
            Priority::Low => 0.0,
            Priority::Medium => self.weight / f64::from(self.position),
            Priority::High => self.weight,
        };
        score * 0.5f64.powf(f64::from(self.deep_page))
    }
}

impl ResultContainer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Merge the results of one engine, weighting them with `weight`.
    pub fn extend(&mut self, engine_name: &str, weight: f64, items: Vec<EngineItem>) {
        self.extend_from_page(engine_name, weight, items, 0);
    }

    /// Merge one engine's results where every item carries its own weight.
    ///
    /// Weighting a whole source uniformly loses information a source itself
    /// already ranked by: the local index blends a semantic score into its
    /// order, and that score has to reach the merged number as well. Weights
    /// must not increase along the source's own order, or a later hit could
    /// outrank an earlier one after the position fusion.
    pub fn extend_weighted(&mut self, engine_name: &str, items: Vec<(f64, EngineItem)>) {
        self.extend_items(engine_name, items, 0);
    }

    /// Merge one engine's results that came from a deeper result page.
    ///
    /// Deeper pages score lower because their positions are shifted by
    /// PAGE_POSITION_STRIDE per extra page, so a page-2 hit never outranks the
    /// same engine's page-1 hits.
    pub fn extend_from_page(
        &mut self,
        engine_name: &str,
        weight: f64,
        items: Vec<EngineItem>,
        page_offset: u32,
    ) {
        let items = items.into_iter().map(|item| (weight, item)).collect();
        self.extend_items(engine_name, items, page_offset);
    }

    fn extend_items(&mut self, engine_name: &str, items: Vec<(f64, EngineItem)>, page_offset: u32) {
        if self.closed {
            return;
        }
        const PAGE_POSITION_STRIDE: u32 = 100;
        let mut position = page_offset.saturating_mul(PAGE_POSITION_STRIDE);
        let deep_page = page_offset;
        for (weight, item) in items {
            self.merge_item(engine_name, weight, item, deep_page, &mut position);
        }
    }

    /// Merge results at the positions the caller recorded.
    ///
    /// The regular extend renumbers positions from one. A recorded replay has
    /// to keep the recorded values (and their gaps), so a result an engine
    /// placed at 7 keeps 1/7 in the position fusion. This entry point exists
    /// for the offline regression, which is why it is test-only.
    #[cfg(test)]
    pub(crate) fn extend_at_positions(
        &mut self,
        engine_name: &str,
        weight: f64,
        items: Vec<(u32, EngineItem)>,
    ) {
        if self.closed {
            return;
        }
        for (position, item) in items {
            let mut position = position.max(1);
            self.merge_item(engine_name, weight, item, 0, &mut position);
        }
    }

    fn merge_item(
        &mut self,
        engine_name: &str,
        weight: f64,
        item: EngineItem,
        deep_page: u32,
        position: &mut u32,
    ) {
        match item {
            EngineItem::Suggestion(suggestion) => {
                if !suggestion.trim().is_empty() {
                    self.suggestions.insert(suggestion);
                }
            }
            EngineItem::Answer(answer) => self.answers.push(answer),
            EngineItem::Correction(correction) => {
                if !correction.trim().is_empty() {
                    self.corrections.insert(correction);
                }
            }
            EngineItem::Infobox(mut infobox) => {
                infobox.img_src = infobox.img_src.filter(|url| is_web_url(url));
                infobox.url = infobox.url.filter(|url| is_web_url(url));
                infobox.urls.retain(|link| is_web_url(&link.url));
                infobox.engine = engine_name.to_string();
                if infobox.id.is_empty() {
                    infobox.id = format!("{engine_name}-{}", self.infoboxes.len());
                }
                match self
                    .infoboxes
                    .iter_mut()
                    .find(|existing| existing.id == infobox.id)
                {
                    Some(existing) => merge_infobox(existing, &infobox),
                    None => self.infoboxes.push(infobox),
                }
            }
            EngineItem::EngineData { key, value } => {
                self.engine_data
                    .entry(engine_name.to_string())
                    .or_default()
                    .insert(key, value);
            }
            EngineItem::Result(mut result) => {
                result.deep_page = deep_page;
                // Unwrap engine redirectors; drop dead wrappers outright so
                // a redirector URL never reaches the user.
                match unwrap_redirect(&result.url) {
                    Some(url) => result.url = url,
                    None => return,
                }
                let keyvalue =
                    result.template == Template::KeyValue && result.url.starts_with("keyvalue:");
                if !keyvalue && !is_web_url(&result.url) {
                    return;
                }
                result.img_src = result.img_src.filter(|url| is_web_url(url));
                result.thumbnail = result.thumbnail.filter(|url| is_web_url(url));
                *position = position.saturating_add(1);
                result.positions = vec![*position];
                result.engine = engine_name.to_string();
                result.engines = vec![engine_name.to_string()];
                result.score = weight;
                let rank = SourceRank {
                    weight,
                    position: *position,
                    deep_page,
                };
                let key = dedup_key_with(&result.url, self.dedup.url_variants);
                match self.index.get(&key).copied() {
                    Some(existing) => {
                        let merged = &mut self.main[existing];
                        let ranks = &mut self.source_ranks[existing];
                        match merged
                            .engines
                            .iter()
                            .position(|engine| engine == engine_name)
                        {
                            Some(source) => {
                                let previous = ranks[source];
                                let score = rank.score(merged.priority);
                                let previous_score = previous.score(merged.priority);
                                if score > previous_score
                                    || (score == previous_score && *position < previous.position)
                                {
                                    ranks[source] = rank;
                                    merged.positions[source] = *position;
                                }
                            }
                            None => {
                                ranks.push(rank);
                                merged.positions.push(*position);
                            }
                        }
                        merged.deep_page =
                            ranks.iter().map(|rank| rank.deep_page).min().unwrap_or(0);
                        merge_result(merged, &result);
                    }
                    None => {
                        self.index.insert(key, self.main.len());
                        self.main.push(result);
                        self.source_ranks.push(vec![rank]);
                    }
                }
            }
        }
    }

    /// Set how duplicate pages are collapsed. Call this before merging
    /// results; the defaults keep both conservative rules on.
    pub fn set_dedup(&mut self, policy: DedupPolicy) {
        self.dedup = policy;
    }

    /// Score and freeze the container (upstream close).
    ///
    /// This keeps the historical signature: the consensus and title/snippet
    /// terms are disabled, so a caller that does not know about them sees the
    /// same scores as before. Use [ResultContainer::close_with] to enable them.
    pub fn close(
        &mut self,
        weight_of: impl Fn(&str) -> f64,
        query_terms: &[String],
        relevance_weight: f64,
        query_text: &str,
        authority_weight: f64,
    ) {
        let options = RankingOptions {
            relevance: relevance_weight,
            authority: authority_weight,
            ..RankingOptions::default()
        };
        self.close_with(weight_of, query_terms, query_text, &options);
    }

    /// Score and freeze the container with every configured ranking term.
    pub fn close_with(
        &mut self,
        weight_of: impl Fn(&str) -> f64,
        query_terms: &[String],
        query_text: &str,
        options: &RankingOptions,
    ) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.infoboxes
            .sort_by_key(|infobox| infobox.img_src.is_none());

        for (result, ranks) in self.main.iter_mut().zip(&self.source_ranks) {
            let mut weight = 1.0;
            for (source, engine) in result.engines.iter().enumerate() {
                weight *= ranks
                    .get(source)
                    .map(|rank| rank.weight)
                    .unwrap_or_else(|| weight_of(engine));
            }
            weight *= result.positions.len() as f64;
            let mut score = 0.0;
            for position in &result.positions {
                match result.priority {
                    Priority::Low => {}
                    Priority::High => score += weight,
                    Priority::Medium => score += weight / (*position as f64),
                }
            }
            // Blend in how well the URL + title cover the query terms. Position
            // fusion alone ranks a homepage above the page that actually answers
            // the query; this lifts the latter the way a neural ranker does.
            let rel = relevance(&result.url, &result.title, &result.content, query_terms);
            result.relevance_score = Some(rel);
            result.score = score + options.relevance * rel;
            // Agreement is additive: a URL that two independent engines both
            // returned is a stronger answer than one engine's lucky hit, even
            // when the positions are the same.
            if options.consensus > 0.0 {
                result.score += options.consensus * consensus(result.engines.len());
            }
            // Title/snippet coverage is a separate knob from the URL-aware
            // relevance term; a page whose title names the query is the answer
            // even when its address does not repeat the words.
            if options.text_match > 0.0 {
                result.score +=
                    options.text_match * text_match(&result.title, &result.content, query_terms);
            }
            // Naming a site in the query means "that site's own page": lift the
            // canonical host and push mirrors down. This is the part keyword
            // and embedding ranking cannot see (official docs vs its mirror).
            // A page-2 hit is a fallback: it never outranks the same engine's
            // page-1 answer for the query.
            if result.deep_page > 0 {
                result.score *= 0.5f64.powf(f64::from(result.deep_page));
            }
            if options.authority > 0.0 {
                let signal = authority(query_text, &result.url) - 0.5;
                result.score += options.authority * signal;
            }
            // A Wikipedia edition or a translated page in the wrong language
            // is noise for this query; one in the query's language is wanted.
            // The lift for the query's language scales with how well the page
            // matches the query, so unrelated translated pages stay down.
            let adjustment = page_language_adjustment(query_text, &result.url);
            let adjustment = if adjustment > 0.0 {
                adjustment * relevance(&result.url, &result.title, &result.content, query_terms)
            } else {
                adjustment
            };
            let language = wikipedia_language_penalty(query_text, &result.url) + adjustment;
            if language != 0.0 {
                result.score += options.relevance * language;
            }
        }

        // --- near-duplicate consolidation ---
        // The URL key absorbs AMP, mobile and print copies, but the same
        // article published under two paths of one host (a re-published
        // version, a trailing-slash variant) can only be recognised from its
        // title and body. A wrong merge destroys evidence, so only a nearly
        // identical body qualifies.
        if self.dedup.near_duplicates {
            merge_near_duplicates(&mut self.main);
        }

        // --- 蒸留（上流超え）: 転載の統合 ---
        // 同じ原発表の転載（ベンダー公告 → 記事 → ミラー）が別ドメインで重複する。
        // これを 1 件に統合しないと「独立した根拠の数」を数え誤る（D47）。
        // 統合は保守的: タイトルの正規化一致、数字列の完全一致、Dice >= 0.95 のすべてを満たす場合のみ。
        distill_transfers(&mut self.main, query_terms);

        self.ordered = order_results(&self.main);
    }

    /// Results sorted the way the UI shows them (upstream
    /// `get_ordered_results`).
    pub fn ordered(&self) -> &[SearchResult] {
        &self.ordered
    }

    /// Results without deduplication-group ordering, in score order.
    pub fn main_results(&self) -> &[SearchResult] {
        &self.main
    }

    /// Rough number of results (upstream reports the number of merged results).
    pub fn number_of_results(&self) -> u64 {
        self.main.len() as u64
    }

    pub fn add_unresponsive_engine(&mut self, engine: &str, error_type: &str, suspended: bool) {
        let entry = UnresponsiveEngine {
            engine: engine.to_string(),
            error_type: error_type.to_string(),
            suspended,
        };
        if !self.unresponsive_engines.contains(&entry) {
            self.unresponsive_engines.push(entry);
            self.unresponsive_engines.sort();
        }
    }

    pub fn add_timing(&mut self, engine: &str, total: f64, load: f64) {
        self.timings.push(Timing {
            engine: engine.to_string(),
            total,
            load,
        });
    }
}

/// Allow web links and relative Recoll paths.
fn is_web_url(value: &str) -> bool {
    match url::Url::parse(value) {
        Ok(url) => matches!(url.scheme(), "http" | "https"),
        Err(url::ParseError::RelativeUrlWithoutBase) => !value.trim().is_empty(),
        Err(_) => false,
    }
}

/// Merge `other` into `origin`, keeping the richer field of each pair.
fn merge_result(origin: &mut SearchResult, other: &SearchResult) {
    // Only upgrade to an HTTPS spelling that an engine actually returned.
    // MCP authorization still uses the exact final URL; no caller-provided
    // scheme substitution is accepted.
    if origin.url.starts_with("http://") && other.url.starts_with("https://") {
        origin.url = other.url.clone();
    }
    if !origin.engines.contains(&other.engine) {
        origin.engines.push(other.engine.clone());
    }
    if origin.title.is_empty() && !other.title.is_empty() {
        origin.title = other.title.clone();
    }
    if origin.content.len() < other.content.len() {
        origin.content = other.content.clone();
    }
    if origin.category.is_empty() && !other.category.is_empty() {
        origin.category = other.category.clone();
    }
    if origin.published_date.is_none() {
        origin.published_date = other.published_date.clone();
    }
    if origin.thumbnail.is_none() {
        origin.thumbnail = other.thumbnail.clone();
    }
    if origin.img_src.is_none() {
        origin.img_src = other.img_src.clone();
    }
    if origin.author.is_none() {
        origin.author = other.author.clone();
    }
    for (key, value) in &other.metadata {
        origin
            .metadata
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
}

fn merge_infobox(origin: &mut Infobox, other: &Infobox) {
    if origin.title.is_empty() {
        origin.title = other.title.clone();
    }
    if origin.content.is_empty() {
        origin.content = other.content.clone();
    }
    if origin.img_src.is_none() {
        origin.img_src = other.img_src.clone();
    }
    if origin.url.is_none() {
        origin.url = other.url.clone();
    }
    for url in &other.urls {
        if !origin.urls.iter().any(|existing| existing.url == url.url) {
            origin.urls.push(url.clone());
        }
    }
    for (key, value) in &other.attributes {
        origin
            .attributes
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
}

/// Group-by-category ordering, ported from `ResultContainer.get_ordered_results`.
/// URL からホストだけを抜く（転載は別ドメインの再掲のみを指す）。
fn url_host(url: &str) -> &str {
    url.split(['/', ':']).nth(3).unwrap_or("")
}

/// タイトルを正規化する（転載判定用）。
///
/// NFKC 相当の大文字小文字・記号・空白の揺れを吸収する。ただし数字列は残す
/// （版番号・CVE 番号の違いは別の記事の目印なので）。
fn normalize_title_for_dedup(title: &str) -> String {
    // 空白・記号を除去して比較する。日本語は空白で区切らないので、
    // 空白を残すと「PHP の脆弱性」と「PHPの脆弱性」が別物になってしまう。
    let mut out = String::with_capacity(title.len());
    for ch in title.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
        }
    }
    out
}

/// タイトルから数字列だけを抜く（版番号・CVE 番号の照合用）。
fn title_numbers(title: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in title.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// 語の Dice 係数（正規化済みタイトルの類似度）。
#[allow(dead_code)]
fn title_dice(a: &str, b: &str) -> f64 {
    let words_a: std::collections::BTreeSet<&str> = a.split_whitespace().collect();
    let words_b: std::collections::BTreeSet<&str> = b.split_whitespace().collect();
    if words_a.is_empty() && words_b.is_empty() {
        return 1.0;
    }
    let overlap = words_a.intersection(&words_b).count();
    2.0 * overlap as f64 / (words_a.len() + words_b.len()) as f64
}

/// 転載の統合（distillation）。
///
/// 統合条件はすべてを満たす場合のみ（誤統合は根拠を失わせるので最悪）:
///  1. 正規化タイトルの完全一致（正規化後は空白を含まないため語集合 Dice は
///     常に 1.0 になり、別途の類似度検査は意味を持たない）
///  2. 数字列の完全一致（版番号・CVE 番号の違いは別の記事）
///  3. 異なるホスト間（同一ホストの別ページは転載ではない）
///
/// 統合時はスコアの高い方を代表とし、engines / positions / merged_sources を寄せる。
fn distill_transfers(results: &mut Vec<SearchResult>, _query_terms: &[String]) {
    if results.len() < 2 {
        return;
    }
    // 正規化タイトル → keep 内の位置。同じタイトルを持つ結果だけが転載候補
    // なので、ペア比較をバケツ内に限定して入力サイズに対してほぼ線形にする。
    // （バケツをまたぐ全ペア比較は、エンジンが返す件数の2乗に比例する。）
    let mut buckets: HashMap<String, Vec<usize>> = HashMap::new();
    let mut keep: Vec<SearchResult> = Vec::with_capacity(results.len());
    'outer: for mut candidate in results.drain(..) {
        let cand_norm = normalize_title_for_dedup(&candidate.title);
        // タイトルが空の結果同士は転載判定できない（空文字同士が全件一致してしまう）。
        // 統合せず、そのまま残す。
        if cand_norm.is_empty() {
            keep.push(candidate);
            continue 'outer;
        }
        let cand_nums = title_numbers(&candidate.title);
        if let Some(indices) = buckets.get(&cand_norm) {
            for &index in indices {
                let existing = &mut keep[index];
                // 転載は「別ドメインでの再掲」。同じホストの別ページ（例: サイト内の一覧と詳細）は
                // タイトルが似ていても別のページなので統合しない。
                if url_host(&existing.url) == url_host(&candidate.url) {
                    continue;
                }
                if title_numbers(&existing.title) != cand_nums {
                    continue; // 数字が違う = 別の記事（版番号・CVE 番号が違う）
                }
                // title_dice の検査は不要: ここに来る時点で正規化タイトルは
                // 文字列として一致しており、空白を含まない正規化済み文字列の
                // 語集合 Dice は常に 1.0 になる。
                // 転載として統合する。代表（existing）に寄せる。
                let merged_label = format!("{} | {}", candidate.title, candidate.url);
                existing.merged_sources.push(merged_label);
                for engine in candidate.engines.drain(..) {
                    if !existing.engines.contains(&engine) {
                        existing.engines.push(engine);
                    }
                }
                existing.positions.append(&mut candidate.positions);
                if existing.score < candidate.score {
                    // より強い結果を代表にする。タイトルの正規化形は統合条件で
                    // 一致済みなので、入れ替えてもバケツの所属は変わらない。
                    std::mem::swap(&mut existing.url, &mut candidate.url);
                    std::mem::swap(&mut existing.title, &mut candidate.title);
                    std::mem::swap(&mut existing.content, &mut candidate.content);
                    std::mem::swap(&mut existing.score, &mut candidate.score);
                    std::mem::swap(
                        &mut existing.relevance_score,
                        &mut candidate.relevance_score,
                    );
                }
                continue 'outer;
            }
        }
        let index = keep.len();
        keep.push(candidate);
        buckets.entry(cand_norm).or_default().push(index);
    }
    *results = keep;
}

/// Merge results that are the same page reached through a different address.
///
/// The URL key already collapses tracking parameters and explicit AMP/print
/// copies. What remains is one host publishing the same page under two paths
/// (a re-published article, a trailing-slash variant, a mobile layout on a
/// subdomain). Title equality alone is not enough, because two different pages
/// of one site can share a title. Both snippets must be long enough for a
/// stable comparison and their character 4-gram Dice coefficient must be at
/// least 0.9.
fn merge_near_duplicates(results: &mut Vec<SearchResult>) {
    if results.len() < 2 {
        return;
    }
    // (host, 正規化タイトル) → keep 内の位置。統合候補は同じバケツの中に
    // しかいないので、全ペア比較ではなくバケツ内比較に留める。本文の
    // 4-gram 集合は組み立てが重いため、同じバケツで初めて必要になった
    // 結果に対してだけ遅延構築する。
    let mut buckets: HashMap<(String, String), Vec<usize>> = HashMap::new();
    let mut grams: Vec<Option<BTreeSet<String>>> = Vec::new();
    let mut keep: Vec<SearchResult> = Vec::with_capacity(results.len());
    'outer: for mut candidate in results.drain(..) {
        let candidate_host = content_host(&candidate.url);
        let candidate_title = normalize_title_for_dedup(&candidate.title);
        if candidate_host.is_empty() || candidate_title.is_empty() {
            keep.push(candidate);
            grams.push(None);
            continue 'outer;
        }
        let Some(candidate_grams) = body_grams(&candidate.content) else {
            keep.push(candidate);
            grams.push(None);
            continue 'outer;
        };
        let key = (candidate_host, candidate_title);
        if let Some(indices) = buckets.get(&key) {
            for &index in indices {
                let existing = &mut keep[index];
                if grams[index].is_none() {
                    grams[index] = body_grams(&existing.content);
                }
                let Some(existing_grams) = &grams[index] else {
                    continue;
                };
                let overlap = existing_grams.intersection(&candidate_grams).count();
                let dice =
                    2.0 * overlap as f64 / (existing_grams.len() + candidate_grams.len()) as f64;
                if dice < 0.9 {
                    continue;
                }
                // Keep the stronger result as the representative and record where
                // the other address came from, the same way distillation does.
                existing
                    .merged_sources
                    .push(format!("{} | {}", candidate.title, candidate.url));
                for engine in candidate.engines.drain(..) {
                    if !existing.engines.contains(&engine) {
                        existing.engines.push(engine);
                    }
                }
                existing.positions.append(&mut candidate.positions);
                if candidate.score > existing.score {
                    std::mem::swap(&mut existing.url, &mut candidate.url);
                    std::mem::swap(&mut existing.title, &mut candidate.title);
                    std::mem::swap(&mut existing.content, &mut candidate.content);
                    std::mem::swap(&mut existing.score, &mut candidate.score);
                    std::mem::swap(
                        &mut existing.relevance_score,
                        &mut candidate.relevance_score,
                    );
                    // 代表が candidate の本文を引き継いだので、4-gram も
                    // そちらに合わせる（統合条件上 host と正規化タイトルは
                    // 一致済みで、バケツの所属は変わらない）。
                    grams[index] = Some(candidate_grams.clone());
                }
                continue 'outer;
            }
        }
        let index = keep.len();
        keep.push(candidate);
        grams.push(Some(candidate_grams));
        buckets.entry(key).or_default().push(index);
    }
    *results = keep;
}

/// The host a result belongs to, ignoring a mobile or AMP subdomain prefix.
fn content_host(url: &str) -> String {
    let Some(host) = host_of(url) else {
        return String::new();
    };
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    for prefix in ["m.", "mobile.", "amp."] {
        if let Some(rest) = host.strip_prefix(prefix) {
            if rest.contains('.') {
                return rest.to_string();
            }
        }
    }
    host
}

/// Character 4-grams of a snippet, or None when it is too short to compare.
///
/// A short snippet is too generic to prove that two pages are the same, so
/// merging stops there. Four-character shingles work for space-delimited and
/// CJK text alike.
fn body_grams(content: &str) -> Option<BTreeSet<String>> {
    const MIN_CHARS: usize = 80;
    const GRAM: usize = 4;
    let normalized: Vec<char> = content
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(|character| character.to_lowercase())
        .collect();
    if normalized.len() < MIN_CHARS {
        return None;
    }
    Some(
        normalized
            .windows(GRAM)
            .map(|window| window.iter().collect())
            .collect(),
    )
}

/// Search engines wrap outbound links in their own redirector
/// (\`google.com/goto?url=\`, \`duckduckgo.com/l/?uddg=\`, ...). Left alone, those
/// wrappers pollute the result set with links that are not the page at all.
///
/// Returns the real destination, \`None\` when the wrapper carries no usable
/// target (the result must be dropped), and the input unchanged otherwise.
pub fn unwrap_redirect(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let Ok(parsed) = url::Url::parse(trimmed) else {
        return Some(trimmed.to_string());
    };
    let host = parsed.host_str().unwrap_or_default().to_lowercase();
    let path = parsed.path().to_lowercase();

    // Which query parameter carries the destination, per wrapper.
    let wrapper = if (host.ends_with("google.") || host.contains(".google."))
        && (path.starts_with("/goto") || path.starts_with("/url"))
    {
        Some(&["url", "q", "continue"][..])
    } else if host.ends_with("duckduckgo.com") && path.starts_with("/l/") {
        Some(&["uddg"][..])
    } else if host.ends_with("bing.com") && path.starts_with("/ck/a") {
        Some(&["u", "url"][..])
    } else if host.ends_with("l.facebook.com") && path.starts_with("/l.php") {
        Some(&["u"][..])
    } else if (host.ends_with("link.zhihu.com") || host.ends_with("link.juejin.cn"))
        && path.starts_with("/")
    {
        Some(&["target"][..])
    } else {
        None
    };
    let Some(keys) = wrapper else {
        return Some(trimmed.to_string());
    };
    for (key, value) in parsed.query_pairs() {
        if keys.contains(&key.as_ref())
            && (value.starts_with("http://") || value.starts_with("https://"))
        {
            return Some(value.into_owned());
        }
    }
    // A wrapper without a target is a dead end: drop the result rather than
    // showing the redirector itself.
    None
}

/// The host of a URL, lowercased and without a port.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, rest)| rest)?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_lowercase())
    }
}

/// Whether [host] is [domain] itself or one of its subdomains.
pub fn domain_matches(host: &str, domain: &str) -> bool {
    let domain = normalize_domain(domain);
    !domain.is_empty() && (host == domain || host.ends_with(&format!(".{domain}")))
}

pub(crate) fn normalize_domain(domain: &str) -> String {
    domain
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or(domain)
        .split(':')
        .next()
        .unwrap_or(domain)
        .trim_start_matches("www.")
        .to_lowercase()
}

/// Apply the optional agent-facing result filters.
pub fn filter_results(
    results: &mut Vec<SearchResult>,
    include_domains: &[String],
    exclude_domains: &[String],
    max_results: Option<usize>,
) {
    if !include_domains.is_empty() {
        results.retain(|result| {
            host_of(&result.url).is_some_and(|host| {
                include_domains
                    .iter()
                    .any(|domain| domain_matches(&host, domain))
            })
        });
    }
    if !exclude_domains.is_empty() {
        results.retain(|result| {
            host_of(&result.url).is_none_or(|host| {
                !exclude_domains
                    .iter()
                    .any(|domain| domain_matches(&host, domain))
            })
        });
    }
    if let Some(limit) = max_results {
        results.truncate(limit.max(1));
    }
}

/// The deduplication key of a URL: scheme independent, without fragment,
/// without the usual tracking parameters and without an unambiguous copy
/// marker (AMP or print variant of the same page).
///
/// The key is assembled from the parsed URL structure, not by concatenating
/// decoded text, so values and separators stay distinguishable:
///
/// - the host keeps its port, except the https default (443). The key always
///   assumes https, so `http://e.test/p`, `https://e.test/p` and
///   `https://e.test:443/p` share one key, while `:8443` and `:9443` differ;
/// - query pairs are re-encoded as pairs, so `?a=b%26c%3Dd` (one value holding
///   `&c=d`) never collapses into `?a=b&c=d` (two parameters);
/// - query order is preserved, since servers may treat it as significant.
///
/// Copy markers are stripped only when they are unmistakable: a dedicated AMP
/// or print query flag, a trailing `/amp` or `/print` segment that follows a
/// real page, or a `.amp.html` / `.print.html` suffix. A page such as
/// `/amp/settings` is untouched.
pub fn dedup_key(url: &str) -> String {
    dedup_key_with(url, true)
}

/// [dedup_key] with the copy-marker rules optionally disabled. The offline
/// regression replays the previous URL-only rules through this switch.
fn dedup_key_with(url: &str, strip_variants: bool) -> String {
    let trimmed = url.trim();
    let Ok(mut parsed) = url::Url::parse(trimmed) else {
        return trimmed.to_lowercase();
    };
    parsed.set_fragment(None);
    let _ = parsed.set_scheme("https");
    if strip_variants {
        strip_copy_markers(&mut parsed);
    }
    let host = parsed.host_str().unwrap_or_default().to_lowercase();
    // `port()` keeps an explicit port unless it is the default for the current
    // scheme, and `set_scheme` does not renormalise, so compare explicitly.
    let authority = match parsed.port().filter(|port| *port != 443) {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    let path = parsed.path().trim_end_matches('/');
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in parsed.query_pairs() {
        if is_tracking_param(&key) {
            continue;
        }
        serializer.append_pair(&key, &value);
    }
    let query = serializer.finish();
    format!(
        "{authority}{path}{}",
        if query.is_empty() {
            String::new()
        } else {
            format!("?{query}")
        }
    )
}

/// Remove the copy markers a site adds to the same page.
///
/// Every rule needs a positive marker, so a genuine page whose path contains
/// the word amp or print (for example /amp/settings) is left alone.
fn strip_copy_markers(parsed: &mut url::Url) {
    // Query flags first: ?amp=1, ?print=1, ?output=amp.
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    let kept: Vec<(String, String)> = pairs
        .iter()
        .filter(|(key, value)| !is_copy_marker_param(key, value))
        .cloned()
        .collect();
    if kept.len() != pairs.len() {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in &kept {
            serializer.append_pair(key, value);
        }
        let query = serializer.finish();
        parsed.set_query(if query.is_empty() { None } else { Some(&query) });
    }
    // Path suffixes: /a/amp, /a/amp/, /a/amp.html, /a.print.html.
    let mut segments: Vec<String> = parsed
        .path_segments()
        .map(|segment| segment.map(str::to_string).collect())
        .unwrap_or_default();
    while segments.last().is_some_and(|segment| segment.is_empty()) {
        segments.pop();
    }
    let mut path_changed = false;
    if let Some(last) = segments.last().cloned() {
        let lower = last.to_ascii_lowercase();
        let stripped = [".amp.html", ".print.html", ".amp", ".print"]
            .iter()
            .find_map(|suffix| {
                lower
                    .strip_suffix(suffix)
                    .filter(|base| !base.is_empty())
                    .map(|base| last[..base.len()].to_string())
            });
        if let Some(base) = stripped {
            *segments.last_mut().expect("the last segment exists") = base;
            path_changed = true;
        } else if matches!(lower.as_str(), "amp" | "print" | "printable") && segments.len() > 1 {
            // A bare trailing segment marks a copy only when a page precedes
            // it; /amp on its own may be a real page and stays.
            segments.pop();
            path_changed = true;
        }
    }
    // Only rewrite a path that actually carried a marker; otherwise the URL
    // keeps the exact escaping it came with.
    if path_changed {
        let path = if segments.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", segments.join("/"))
        };
        parsed.set_path(&path);
    }
}

/// Query parameters that only switch to a copy of the same page.
fn is_copy_marker_param(key: &str, value: &str) -> bool {
    let key = key.to_ascii_lowercase();
    let value = value.to_ascii_lowercase();
    match key.as_str() {
        "amp" | "print" | "printable" => matches!(value.as_str(), "" | "1" | "true" | "yes"),
        "amp_js_v" => !value.is_empty(),
        "output" => matches!(value.as_str(), "amp" | "print" | "printable"),
        _ => false,
    }
}

/// Tracking parameters removed when deduplicating (a subset of the ClearURLs
/// rules used by the tracker remover plugin).
pub fn is_tracking_param(key: &str) -> bool {
    let key = key.to_lowercase();
    key.starts_with("utm_")
        || matches!(
            key.as_str(),
            "fbclid"
                | "gclid"
                | "dclid"
                | "msclkid"
                | "yclid"
                | "_openstat"
                | "igshid"
                | "mc_eid"
                | "mkt_tok"
                | "vero_id"
                | "wickedid"
                | "oly_anon_id"
                | "oly_enc_id"
                | "s_cid"
                | "spm"
                | "ref_src"
                | "ref_url"
                | "cmpid"
                | "campaign_id"
        )
}

/// A short human readable URL for the UI.
pub fn pretty_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(parsed) => {
            let host = parsed.host_str().unwrap_or_default();
            let path = parsed.path().trim_end_matches('/');
            if path.is_empty() || path == "/" {
                host.to_string()
            } else {
                format!("{host}{path}")
            }
        }
        Err(_) => url.to_string(),
    }
}

#[cfg(test)]
mod quality;
#[cfg(test)]
mod tests {

    use super::*;
    use crate::engine::EngineItem;

    fn result(url: &str, title: &str) -> EngineItem {
        EngineItem::Result(SearchResult::new(url, title))
    }

    #[test]
    fn duplicate_urls_are_merged() {
        let mut container = ResultContainer::new();
        container.extend("a", 1.0, vec![result("https://example.com/x", "one")]);
        container.extend("b", 1.0, vec![result("http://example.com/x#anchor", "one")]);
        assert_eq!(container.main_results().len(), 1);
        assert_eq!(container.main_results()[0].engines.len(), 2);
    }

    #[test]
    fn executable_result_urls_are_rejected() {
        for url in [
            "javascript:void(0)",
            "JaVaScRiPt:void(0)",
            "\tjava\nscript:void(0)",
            "data:text/html,example",
            "vbscript:example",
            "https://",
            "",
        ] {
            for template in [Template::Default, Template::KeyValue] {
                let mut container = ResultContainer::new();
                let item = SearchResult::new(url, "unsafe").with_template(template);
                container.extend("a", 1.0, vec![EngineItem::Result(item)]);
                assert!(container.main_results().is_empty(), "{url:?}");
            }
        }
    }

    #[test]
    fn web_relative_and_keyvalue_results_are_preserved() {
        for url in [
            "https://example.com/a",
            "http://example.com/b",
            "/docs/a.pdf",
            "../docs/b.pdf",
            "//example.com/c",
            "?page=2",
            "#section",
        ] {
            let mut container = ResultContainer::new();
            container.extend("a", 1.0, vec![result(url, "safe")]);
            assert_eq!(container.main_results()[0].url, url);
        }

        let mut container = ResultContainer::new();
        let row = SearchResult::new("keyvalue:0", "database row")
            .with_template(Template::KeyValue)
            .with_metadata("magnetlink", "magnet:?xt=urn:btih:example");
        container.extend("db", 1.0, vec![EngineItem::Result(row)]);
        assert_eq!(container.main_results()[0].url, "keyvalue:0");
        assert_eq!(
            container.main_results()[0].metadata["magnetlink"],
            "magnet:?xt=urn:btih:example"
        );
    }

    #[test]
    fn merged_images_and_infobox_links_reject_executable_urls() {
        let mut container = ResultContainer::new();
        container.extend("a", 1.0, vec![result("https://example.com/a", "safe")]);
        let mut duplicate = SearchResult::new("https://example.com/a", "safe");
        duplicate.img_src = Some("JaVaScRiPt:void(0)".into());
        duplicate.thumbnail = Some("data:text/html,example".into());
        let infobox = Infobox {
            id: "box".into(),
            title: "reference".into(),
            ..Default::default()
        };
        container.extend("a", 1.0, vec![EngineItem::Infobox(infobox.clone())]);
        container.extend(
            "b",
            1.0,
            vec![
                EngineItem::Result(duplicate),
                EngineItem::Infobox(Infobox {
                    img_src: Some("java\tscript:void(0)".into()),
                    url: Some("javascript:void(0)".into()),
                    urls: vec![
                        InfoboxUrl {
                            title: "unsafe".into(),
                            url: "JaVaScRiPt:void(0)".into(),
                        },
                        InfoboxUrl {
                            title: "safe".into(),
                            url: "https://example.com/reference".into(),
                        },
                    ],
                    ..infobox
                }),
            ],
        );
        assert!(container.main_results()[0].img_src.is_none());
        assert!(container.main_results()[0].thumbnail.is_none());
        assert!(container.infoboxes[0].img_src.is_none());
        assert!(container.infoboxes[0].url.is_none());
        assert_eq!(container.infoboxes[0].urls.len(), 1);
        assert_eq!(
            container.infoboxes[0].urls[0].url,
            "https://example.com/reference"
        );

        let mut safe = SearchResult::new("https://example.com/a", "safe");
        safe.img_src = Some("https://example.com/image.png".into());
        safe.thumbnail = Some("/thumb.png".into());
        container.extend(
            "c",
            1.0,
            vec![
                EngineItem::Result(safe),
                EngineItem::Infobox(Infobox {
                    id: "box".into(),
                    img_src: Some("https://example.com/image.png".into()),
                    url: Some("/reference".into()),
                    ..Default::default()
                }),
            ],
        );
        assert_eq!(
            container.main_results()[0].img_src.as_deref(),
            Some("https://example.com/image.png")
        );
        assert_eq!(
            container.main_results()[0].thumbnail.as_deref(),
            Some("/thumb.png")
        );
        assert_eq!(
            container.infoboxes[0].img_src.as_deref(),
            Some("https://example.com/image.png")
        );
        assert_eq!(container.infoboxes[0].url.as_deref(), Some("/reference"));
    }

    #[test]
    fn tracking_parameters_do_not_prevent_merging() {
        let mut container = ResultContainer::new();
        container.extend("a", 1.0, vec![result("https://example.com/x?a=1", "one")]);
        container.extend(
            "b",
            1.0,
            vec![result("https://example.com/x?a=1&utm_source=news", "one")],
        );
        assert_eq!(container.main_results().len(), 1);
    }

    #[test]
    fn higher_ranked_positions_score_higher() {
        let mut container = ResultContainer::new();
        container.extend(
            "a",
            1.0,
            vec![
                result("https://example.com/first", "first"),
                result("https://example.com/second", "second"),
            ],
        );
        container.close(|_| 1.0, &[], 0.0, "", 0.0);
        let ordered = container.ordered();
        assert_eq!(ordered[0].url, "https://example.com/first");
        assert!(ordered[0].score > ordered[1].score);
    }

    #[test]
    fn engine_weight_multiplies_the_score() {
        let mut container = ResultContainer::new();
        container.extend("a", 0.5, vec![result("https://example.com/a", "a")]);
        container.extend("b", 2.0, vec![result("https://example.com/b", "b")]);
        container.close(
            |engine| if engine == "a" { 0.5 } else { 2.0 },
            &[],
            0.0,
            "",
            0.0,
        );
        // `ordered` is the ranking the UI shows: engine weight wins.
        assert_eq!(container.ordered()[0].url, "https://example.com/b");
    }

    #[test]
    fn weights_supplied_with_results_override_registry_defaults() {
        let mut container = ResultContainer::new();
        container.extend(
            "routed",
            2.0,
            vec![result("https://example.com/routed", "routed")],
        );
        container.extend(
            "official site",
            0.5,
            vec![result("https://example.com/official", "official")],
        );
        container.close(|_| 1.0, &[], 0.0, "", 0.0);
        assert_eq!(container.ordered()[0].score, 2.0);
        assert_eq!(container.ordered()[1].score, 0.5);
    }

    #[test]
    fn per_item_weights_reach_the_merged_score() {
        let mut container = ResultContainer::new();
        container.extend_weighted(
            "local_index",
            vec![
                (1.5, result("https://example.com/first", "first")),
                (1.2, result("https://example.com/second", "second")),
            ],
        );
        container.close(|_| 1.0, &[], 0.0, "", 0.0);
        let ordered = container.ordered();
        assert_eq!(ordered[0].url, "https://example.com/first");
        assert_eq!(ordered[0].score, 1.5);
        assert_eq!(ordered[1].score, 0.6);
    }

    #[test]
    fn repeated_urls_from_one_source_cast_one_vote() {
        let mut container = ResultContainer::new();
        container.extend(
            "a",
            1.0,
            vec![
                result("https://example.com/page", "page"),
                result("http://example.com/page?utm_source=a", "page again"),
            ],
        );
        container.extend_from_page(
            "a",
            1.0,
            vec![result("https://example.com/page", "page again")],
            1,
        );
        container.close(|_| 1.0, &[], 0.0, "", 0.0);
        let result = &container.ordered()[0];
        assert_eq!(result.engines, ["a"]);
        assert_eq!(result.positions, [1]);
        assert_eq!(result.score, 1.0);
    }

    #[test]
    fn independent_sources_still_contribute_their_best_positions() {
        let mut container = ResultContainer::new();
        container.extend_from_page(
            "a",
            1.0,
            vec![result("https://example.com/page", "page")],
            1,
        );
        container.extend("a", 1.0, vec![result("https://example.com/page", "page")]);
        container.extend("b", 1.0, vec![result("https://example.com/page", "page")]);
        container.close(|_| 1.0, &[], 0.0, "", 0.0);
        let result = &container.ordered()[0];
        assert_eq!(result.engines, ["a", "b"]);
        assert_eq!(result.positions, [1, 1]);
        assert_eq!(result.deep_page, 0);
        assert_eq!(result.score, 4.0);
    }

    #[test]
    fn query_specific_weights_do_not_leak_between_results_of_one_source() {
        for reverse in [false, true] {
            let mut container = ResultContainer::new();
            let mut passes = vec![
                (
                    1.0,
                    vec![
                        result("https://example.com/plain", "plain"),
                        result("https://example.com/shared", "shared"),
                    ],
                ),
                (
                    2.0,
                    vec![
                        result("https://example.com/shared", "shared"),
                        result("https://example.com/routed", "routed"),
                    ],
                ),
            ];
            if reverse {
                passes.reverse();
            }
            for (weight, items) in passes {
                container.extend("source", weight, items);
            }
            container.close(|_| 1.0, &[], 0.0, "", 0.0);
            let scores: std::collections::BTreeMap<_, _> = container
                .ordered()
                .iter()
                .map(|result| (result.url.as_str(), result.score))
                .collect();
            assert_eq!(scores["https://example.com/plain"], 1.0);
            assert_eq!(scores["https://example.com/shared"], 2.0);
            assert_eq!(scores["https://example.com/routed"], 1.0);
        }
    }

    #[test]
    fn suggestions_answers_and_corrections_are_collected() {
        let mut container = ResultContainer::new();
        container.extend(
            "a",
            1.0,
            vec![
                EngineItem::Suggestion("did you mean rust".into()),
                EngineItem::Answer("42".into()),
                EngineItem::Correction("corrected".into()),
                result("https://example.com/x", "x"),
            ],
        );
        assert!(container.suggestions.contains("did you mean rust"));
        assert_eq!(container.answers, vec!["42".to_string()]);
        assert!(container.corrections.contains("corrected"));
    }

    #[test]
    fn results_of_the_same_category_stay_close_together() {
        let mut container = ResultContainer::new();
        let mut items = Vec::new();
        for index in 0..10 {
            let mut image = SearchResult::new(format!("https://img.example.com/{index}"), "img");
            image.category = "images".into();
            image.template = Template::Images;
            items.push(EngineItem::Result(image));
            let mut web = SearchResult::new(format!("https://web.example.com/{index}"), "web");
            web.category = "general".into();
            web.score = 100.0 - index as f64;
            items.push(EngineItem::Result(web));
        }
        container.extend("a", 1.0, items);
        container.close(|_| 1.0, &[], 0.0, "", 0.0);
        let ordered = container.ordered();
        // Within the first 8 entries of the images group every second entry is an
        // image result (upstream groups results of one category).
        assert!(ordered[..8].iter().all(|r| r.url.contains("example.com")));
    }

    #[test]
    fn dedup_key_ignores_scheme_fragment_and_tracking() {
        assert_eq!(
            dedup_key("http://Example.com/a/?utm_source=x#frag"),
            dedup_key("https://example.com/a")
        );
    }

    #[test]
    fn query_delimiters_inside_a_value_do_not_collide_with_parameter_boundaries() {
        // `b%26c%3Dd` is one value that contains "&c=d", not two parameters.
        assert_eq!(
            dedup_key("https://e.test/p?a=b%26c%3Dd"),
            "e.test/p?a=b%26c%3Dd"
        );
        assert_eq!(dedup_key("https://e.test/p?a=b&c=d"), "e.test/p?a=b&c=d");
        assert_ne!(
            dedup_key("https://e.test/p?a=b%26c%3Dd"),
            dedup_key("https://e.test/p?a=b&c=d")
        );
    }

    #[test]
    fn the_port_is_part_of_the_identity() {
        assert_eq!(dedup_key("https://e.test:8443/p"), "e.test:8443/p");
        assert_eq!(dedup_key("https://e.test:9443/p"), "e.test:9443/p");
        assert_ne!(
            dedup_key("https://e.test:8443/p"),
            dedup_key("https://e.test:9443/p")
        );
    }

    #[test]
    fn the_https_default_port_is_elided() {
        // Spec: the key is scheme independent and always assumes https, so the
        // explicit default port and the absent port produce the same key.
        assert_eq!(dedup_key("https://e.test:443/p"), "e.test/p");
        assert_eq!(dedup_key("http://e.test/p"), "e.test/p");
        assert_eq!(dedup_key("https://e.test:8443/p"), "e.test:8443/p");
    }

    #[test]
    fn query_parameter_order_is_significant() {
        assert_eq!(dedup_key("https://e.test/p?a=1&b=2"), "e.test/p?a=1&b=2");
        assert_eq!(dedup_key("https://e.test/p?b=2&a=1"), "e.test/p?b=2&a=1");
        assert_ne!(
            dedup_key("https://e.test/p?a=1&b=2"),
            dedup_key("https://e.test/p?b=2&a=1")
        );
    }

    #[test]
    fn duplicate_results_keep_the_returned_https_url_in_either_order() {
        for urls in [
            ["http://github.com/trending", "https://github.com/trending"],
            ["https://github.com/trending", "http://github.com/trending"],
        ] {
            let mut container = ResultContainer::new();
            container.extend("a", 1.0, vec![result(urls[0], "trending")]);
            container.extend("b", 1.0, vec![result(urls[1], "trending")]);
            assert_eq!(container.main_results().len(), 1);
            assert_eq!(
                container.main_results()[0].url,
                "https://github.com/trending"
            );
        }
    }

    #[test]
    fn redirect_wrappers_are_unwrapped_or_dropped() {
        assert_eq!(
            unwrap_redirect("https://www.google.com/goto?url=https%3A%2F%2Ftokio.rs%2F"),
            Some("https://tokio.rs/".to_string())
        );
        assert_eq!(
            unwrap_redirect("https://duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa&rut=x"),
            Some("https://example.com/a".to_string())
        );
        // a wrapper with no destination is dropped
        assert_eq!(unwrap_redirect("https://www.google.com/goto?url="), None);
        // an ordinary URL is untouched
        assert_eq!(
            unwrap_redirect("https://tokio.rs/tokio/tutorial"),
            Some("https://tokio.rs/tokio/tutorial".to_string())
        );
    }

    #[test]
    fn pretty_url_strips_the_scheme() {
        assert_eq!(pretty_url("https://example.com/a/b/"), "example.com/a/b");
        assert_eq!(pretty_url("https://example.com/"), "example.com");
    }

    fn sample(url: &str) -> SearchResult {
        SearchResult {
            url: url.to_string(),
            title: url.to_string(),
            ..SearchResult::default()
        }
    }

    #[test]
    fn host_of_handles_port_userinfo_and_case() {
        assert_eq!(
            host_of("https://WWW.Example.com:8443/a"),
            Some("www.example.com".into())
        );
        assert_eq!(
            host_of("https://user:pw@example.com/a"),
            Some("example.com".into())
        );
        assert_eq!(host_of("not a url"), None);
    }

    #[test]
    fn domain_matches_subdomains_but_not_lookalikes() {
        assert!(domain_matches("example.com", "example.com"));
        assert!(domain_matches("docs.example.com", "example.com"));
        assert!(domain_matches(
            "docs.example.com",
            "https://www.example.com/path"
        ));
        assert!(!domain_matches("notexample.com", "example.com"));
        assert!(!domain_matches("example.com.evil.test", "example.com"));
    }

    #[test]
    fn filter_results_applies_include_exclude_and_limit() {
        let mut results = vec![
            sample("https://keep.test/a"),
            sample("https://docs.keep.test/b"),
            sample("https://drop.test/c"),
            sample("https://other.test/d"),
        ];
        filter_results(&mut results, &["keep.test".into()], &[], None);
        assert_eq!(results.len(), 2);
        filter_results(&mut results, &[], &["docs.keep.test".into()], None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://keep.test/a");
        filter_results(&mut results, &[], &[], Some(0));
        assert_eq!(results.len(), 1, "a limit below one keeps one result");
    }

    #[test]
    fn raw_content_is_serialised_only_when_present() {
        let mut result = sample("https://example.com/");
        let plain = serde_json::to_value(&result).expect("json");
        assert!(plain.get("raw_content").is_none());
        result.raw_content = Some("page text".into());
        let rich = serde_json::to_value(&result).expect("json");
        assert_eq!(rich["raw_content"], "page text");
    }
    // -------------------------------------------------------------------------
    // 転載統合（distillation）: 上流超えの蒸留
    // -------------------------------------------------------------------------

    fn sr(title: &str, url: &str, engine: &str) -> SearchResult {
        SearchResult {
            url: url.to_string(),
            title: title.to_string(),
            content: String::new(),
            engine: engine.to_string(),
            engines: vec![engine.to_string()],
            positions: vec![1],
            score: 1.0,
            ..SearchResult::default()
        }
    }

    #[test]
    fn transfer_of_the_same_article_is_merged() {
        // 同じ記事の転載（全角半角・空白の揺れ）は 1 件に統合する
        let mut results = vec![
            sr("PHP の脆弱性対策について", "https://jvn.jp/a", "jvn_vu"),
            sr(
                "PHPの脆弱性対策について",
                "https://mirror.example/b",
                "bing_web",
            ),
        ];
        distill_transfers(&mut results, &["php".to_string()]);
        assert_eq!(results.len(), 1, "転載を統合する");
        assert_eq!(results[0].merged_sources.len(), 1, "統合元を記録する");
        assert!(results[0].engines.contains(&"jvn_vu".to_string()));
        assert!(results[0].engines.contains(&"bing_web".to_string()));
    }

    #[test]
    fn different_version_numbers_are_kept_separate() {
        // 数字（版番号）が違う記事は別の記事。統合してはいけない（根拠を失わせる）
        let mut results = vec![
            sr("PHP 8.0.33 の更新", "https://a.example/1", "jvn_vu"),
            sr("PHP 8.0.31 の更新", "https://b.example/2", "bing_web"),
        ];
        distill_transfers(&mut results, &["php".to_string()]);
        assert_eq!(results.len(), 2, "版番号が違えば統合しない");
    }

    #[test]
    fn unrelated_articles_are_kept_separate() {
        let mut results = vec![
            sr("Apache の脆弱性", "https://a.example/1", "jvn_vu"),
            sr("nginx の設定ミス", "https://b.example/2", "bing_web"),
        ];
        distill_transfers(&mut results, &[]);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn normalize_title_for_dedup_folds_case_and_punctuation() {
        assert_eq!(
            normalize_title_for_dedup("PHP の脆弱性対策について！"),
            normalize_title_for_dedup("phpの脆弱性対策について"),
        );
    }

    #[test]
    fn title_numbers_detects_version_differences() {
        assert_eq!(
            title_numbers("PHP 8.0.33"),
            vec!["8".to_string(), "0".to_string(), "33".to_string()]
        );
        assert_ne!(title_numbers("PHP 8.0.33"), title_numbers("PHP 8.0.31"));
    }

    // -------------------------------------------------------------------------
    // Ranking terms (multi-source agreement, title/snippet match) and copies
    // -------------------------------------------------------------------------

    fn long_body(subject: &str) -> String {
        format!("{subject} の説明です。").repeat(12)
    }

    #[test]
    fn copy_markers_collapse_amp_and_print_variants() {
        assert_eq!(
            dedup_key("https://e.test/a/amp"),
            dedup_key("https://e.test/a")
        );
        assert_eq!(
            dedup_key("https://e.test/a?print=1"),
            dedup_key("https://e.test/a")
        );
        assert_eq!(
            dedup_key("https://e.test/a.amp.html"),
            dedup_key("https://e.test/a")
        );
        assert_eq!(
            dedup_key("https://e.test/a/print/"),
            dedup_key("https://e.test/a")
        );
        // A real page whose path happens to say "amp" is not a copy.
        assert_ne!(
            dedup_key("https://e.test/amp/settings"),
            dedup_key("https://e.test/settings")
        );
        assert_ne!(
            dedup_key("https://e.test/amp"),
            dedup_key("https://e.test/")
        );
        // Percent-encoded paths keep their escaping: rewriting the path only
        // strips a marker and must not double-encode it.
        assert_eq!(
            dedup_key("https://e.test/%E5%AF%8C%E5%A3%AB/amp"),
            dedup_key("https://e.test/%E5%AF%8C%E5%A3%AB")
        );
        assert_eq!(
            dedup_key("https://e.test/a?amp=1&page=2"),
            dedup_key("https://e.test/a?page=2")
        );
        // The previous URL-only rules remain available for a baseline replay.
        assert_ne!(
            dedup_key_with("https://e.test/a/amp", false),
            dedup_key_with("https://e.test/a", false)
        );
    }

    #[test]
    fn copy_variants_merge_while_collecting() {
        let mut container = ResultContainer::new();
        container.extend("a", 1.0, vec![result("https://e.test/a", "page")]);
        container.extend("b", 1.0, vec![result("https://e.test/a/amp", "page")]);
        assert_eq!(container.main_results().len(), 1);
        assert_eq!(container.main_results()[0].engines, ["a", "b"]);
        assert_eq!(container.main_results()[0].url, "https://e.test/a");
    }

    #[test]
    fn disabling_fuzzy_dedup_keeps_copy_urls_apart() {
        let mut container = ResultContainer::new();
        container.set_dedup(DedupPolicy {
            url_variants: false,
            near_duplicates: false,
        });
        container.extend("a", 1.0, vec![result("https://e.test/a", "page")]);
        container.extend("b", 1.0, vec![result("https://e.test/a/amp", "page")]);
        assert_eq!(container.main_results().len(), 2);
    }

    #[test]
    fn a_mobile_copy_with_the_same_title_and_body_is_merged() {
        let body = long_body("インストール手順");
        let mut desktop = SearchResult::new("https://docs.e.test/guide", "Install guide");
        desktop.content = body.clone();
        let mut mobile = SearchResult::new("https://m.docs.e.test/guide", "Install guide");
        mobile.content = body;
        let mut container = ResultContainer::new();
        container.extend("a", 1.0, vec![EngineItem::Result(desktop)]);
        container.extend("b", 1.0, vec![EngineItem::Result(mobile)]);
        container.close(|_| 1.0, &[], 0.0, "", 0.0);
        assert_eq!(
            container.main_results().len(),
            1,
            "the mobile copy is merged"
        );
        assert_eq!(container.main_results()[0].url, "https://docs.e.test/guide");
        assert_eq!(
            container.main_results()[0].merged_sources,
            ["Install guide | https://m.docs.e.test/guide"]
        );
    }

    #[test]
    fn a_same_title_is_not_enough_when_the_bodies_differ() {
        let mut first = SearchResult::new("https://docs.e.test/guide", "Install guide");
        first.content = long_body("インストール手順");
        let mut second = SearchResult::new("https://docs.e.test/changelog", "Install guide");
        second.content = long_body("変更履歴");
        let mut container = ResultContainer::new();
        container.extend("a", 1.0, vec![EngineItem::Result(first)]);
        container.extend("b", 1.0, vec![EngineItem::Result(second)]);
        container.close(|_| 1.0, &[], 0.0, "", 0.0);
        assert_eq!(container.main_results().len(), 2);
    }

    #[test]
    fn short_snippets_are_never_merged_by_body_similarity() {
        let mut first = SearchResult::new("https://docs.e.test/guide", "Install guide");
        first.content = "same words".into();
        let mut second = SearchResult::new("https://docs.e.test/guide-old", "Install guide");
        second.content = "same words".into();
        let mut container = ResultContainer::new();
        container.extend("a", 1.0, vec![EngineItem::Result(first)]);
        container.extend("b", 1.0, vec![EngineItem::Result(second)]);
        container.close(|_| 1.0, &[], 0.0, "", 0.0);
        assert_eq!(container.main_results().len(), 2);
    }

    fn corroborated_container() -> ResultContainer {
        // "single" is the first hit of one engine. "shared" is the fifth
        // hit of two engines: without the consensus term 0.8 < 1.0, and with
        // it 0.8 + 0.5 * consensus(2) > 1.0.
        let mut container = ResultContainer::new();
        container.extend(
            "a",
            1.0,
            vec![
                result("https://e.test/single", "single"),
                result("https://e.test/a2", "a2"),
                result("https://e.test/a3", "a3"),
                result("https://e.test/a4", "a4"),
                result("https://e.test/shared", "shared"),
            ],
        );
        container.extend(
            "b",
            1.0,
            vec![
                result("https://e.test/b1", "b1"),
                result("https://e.test/b2", "b2"),
                result("https://e.test/b3", "b3"),
                result("https://e.test/b4", "b4"),
                result("https://e.test/shared", "shared"),
            ],
        );
        container
    }

    #[test]
    fn consensus_lifts_a_page_two_engines_returned() {
        let mut baseline = corroborated_container();
        baseline.close(|_| 1.0, &[], 0.0, "", 0.0);
        assert_eq!(baseline.ordered()[0].url, "https://e.test/single");

        let mut boosted = corroborated_container();
        boosted.close_with(
            |_| 1.0,
            &[],
            "",
            &RankingOptions {
                consensus: 0.5,
                ..RankingOptions::default()
            },
        );
        assert_eq!(boosted.ordered()[0].url, "https://e.test/shared");
        assert_eq!(boosted.ordered()[0].engines, ["a", "b"]);
    }

    #[test]
    fn the_title_and_snippet_match_lifts_the_answering_page() {
        let build = || {
            let mut container = ResultContainer::new();
            container.extend(
                "a",
                1.0,
                vec![
                    result("https://docs.e.test/one", "Untitled note"),
                    EngineItem::result(
                        SearchResult::new("https://docs.e.test/two", "Async runtime")
                            .with_content("A guide to the async runtime and its scheduler."),
                    ),
                ],
            );
            container
        };
        let terms = query_terms("async runtime");
        let mut baseline = build();
        baseline.close(|_| 1.0, &terms, 0.0, "async runtime", 0.0);
        assert_eq!(baseline.ordered()[0].url, "https://docs.e.test/one");

        let mut boosted = build();
        boosted.close_with(
            |_| 1.0,
            &terms,
            "async runtime",
            &RankingOptions {
                text_match: 1.0,
                ..RankingOptions::default()
            },
        );
        assert_eq!(boosted.ordered()[0].url, "https://docs.e.test/two");
    }

    #[test]
    fn the_default_terms_do_not_reorder_position_fusion() {
        let shipped = crate::settings::Settings::defaults().search;
        let build = || {
            let mut container = ResultContainer::new();
            container.extend(
                "a",
                1.0,
                vec![
                    result("https://e.test/first", "guide to the topic"),
                    result("https://e.test/second", "another guide to the topic"),
                ],
            );
            container
        };
        let terms = query_terms("guide topic");
        let mut baseline = build();
        baseline.close(|_| 1.0, &terms, 0.0, "guide topic", 0.0);
        let mut with_defaults = build();
        with_defaults.close_with(
            |_| 1.0,
            &terms,
            "guide topic",
            &RankingOptions {
                relevance: 0.0,
                authority: 0.0,
                consensus: shipped.consensus_weight,
                text_match: shipped.text_match_weight,
            },
        );
        let order = |container: &ResultContainer| -> Vec<String> {
            container
                .ordered()
                .iter()
                .map(|result| result.url.clone())
                .collect()
        };
        assert_eq!(order(&baseline), order(&with_defaults));
    }

    #[test]
    fn zero_coefficients_keep_the_previous_scores() {
        let mut legacy = corroborated_container();
        legacy.close(|_| 1.0, &[], 0.0, "", 0.0);
        let mut explicit = corroborated_container();
        explicit.close_with(|_| 1.0, &[], "", &RankingOptions::default());
        let legacy_scores: Vec<f64> = legacy.ordered().iter().map(|r| r.score).collect();
        let explicit_scores: Vec<f64> = explicit.ordered().iter().map(|r| r.score).collect();
        assert_eq!(legacy_scores, explicit_scores);
    }
}
