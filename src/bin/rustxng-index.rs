//! Crawl a seed list into the local index, and query it from the terminal.
//!
//!   rustxng-index crawl --seeds seeds.txt --out ./index-data --max-pages 2000
//!       [--sitemaps] [--within-seed-path] [--skip-indexed] [--max-per-host N] [--workers N]
//!   rustxng-index search --index ./index-data "tokio runtime"
//!   rustxng-index stats --index ./index-data
//!
//! The crawler is a polite one: it announces itself with a stable User-Agent,
//! obeys robots.txt (RFC 9309, including `Crawl-delay`), `noindex`/`nofollow`
//! and `rel=nofollow`, never leaves the host it was started on, spaces its
//! requests per host, backs off when a host answers 429/503, and bounds every
//! download.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustxng::crawl::{self, Directives};
use rustxng::localindex::{LocalIndex, Page, has_cjk};
use rustxng::robots::Robots;
use scraper::{Html, Selector};

/// Skip files that are not pages.
const SKIP_EXTENSIONS: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".ico", ".css", ".js", ".json", ".zip",
    ".gz", ".tar", ".pdf", ".mp4", ".mp3", ".woff", ".woff2", ".ttf", ".xml", ".rss", ".txt",
];

/// Most bytes read from one page. Larger documents are not articles.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Most redirects followed for one request, always within the same host.
const MAX_REDIRECTS: usize = 5;
/// Longest pause after a 429/503 without a usable `Retry-After`.
const DEFAULT_BACKOFF: Duration = Duration::from_secs(30);
/// Longest `Retry-After` we are willing to honour.
const MAX_BACKOFF: Duration = Duration::from_secs(300);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().cloned() else {
        eprintln!("usage: rustxng-index <crawl|search|stats> [options]");
        std::process::exit(2);
    };
    let options = Options::parse(&args[1..]);
    let result = match command.as_str() {
        "crawl" => crawl(&options),
        "search" => search(&options),
        "stats" => stats(&options),
        other => {
            eprintln!("unknown command: {other}");
            std::process::exit(2);
        }
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

/// Long form flags, kept tiny so the binary has no extra dependencies.
struct Options {
    seeds: Option<PathBuf>,
    index: PathBuf,
    max_pages: usize,
    max_depth: usize,
    /// Pause between two requests to one host (a host's `Crawl-delay` wins when
    /// it is longer).
    delay_ms: u64,
    workers: usize,
    use_sitemaps: bool,
    /// Stay under the directory of the seed URLs (per host).
    within_seed_path: bool,
    /// Do not download pages the index already holds.
    skip_indexed: bool,
    /// Most pages taken from one host, so no single site fills the index.
    max_per_host: usize,
    /// Mail address or URL where the crawled sites can reach us.
    contact: String,
    limit: usize,
    query: Vec<String>,
}

impl Options {
    fn parse(args: &[String]) -> Self {
        let mut options = Self {
            seeds: None,
            index: PathBuf::from("index-data"),
            max_pages: 500,
            max_depth: 2,
            delay_ms: 1000,
            workers: 4,
            use_sitemaps: false,
            within_seed_path: false,
            skip_indexed: false,
            max_per_host: 2000,
            contact: String::new(),
            limit: 10,
            query: Vec::new(),
        };
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            let mut next = || {
                index += 1;
                args.get(index).cloned()
            };
            match arg.as_str() {
                "--seeds" => options.seeds = next().map(PathBuf::from),
                "--index" | "--out" => {
                    if let Some(value) = next() {
                        options.index = PathBuf::from(value);
                    }
                }
                "--max-pages" => {
                    if let Some(value) = next().and_then(|v| v.parse::<usize>().ok()) {
                        options.max_pages = value;
                    }
                }
                "--max-depth" => {
                    if let Some(value) = next().and_then(|v| v.parse::<usize>().ok()) {
                        options.max_depth = value;
                    }
                }
                "--delay-ms" => {
                    if let Some(value) = next().and_then(|v| v.parse::<u64>().ok()) {
                        options.delay_ms = value;
                    }
                }
                "--max-per-host" => {
                    if let Some(value) = next().and_then(|v| v.parse::<usize>().ok()) {
                        options.max_per_host = value.max(1);
                    }
                }
                "--contact" => {
                    if let Some(value) = next() {
                        options.contact = value;
                    }
                }
                "--sitemaps" => options.use_sitemaps = true,
                "--within-seed-path" => options.within_seed_path = true,
                "--skip-indexed" => options.skip_indexed = true,
                "--workers" => {
                    if let Some(value) = next().and_then(|v| v.parse::<usize>().ok()) {
                        options.workers = value.clamp(1, 32);
                    }
                }
                "--limit" => {
                    if let Some(value) = next().and_then(|v| v.parse::<usize>().ok()) {
                        options.limit = value;
                    }
                }
                other if !other.starts_with("--") => options.query.push(other.to_string()),
                _ => {}
            }
            index += 1;
        }
        options
    }
}

fn search(options: &Options) -> Result<(), String> {
    let index = LocalIndex::open(&options.index)?;
    let query = options.query.join(" ");
    if query.is_empty() {
        return Err("no query given".to_string());
    }
    let started = Instant::now();
    let hits = index.search(&query, options.limit)?;
    let elapsed = started.elapsed();
    println!(
        "{} hits in {:.1} ms (index has {} documents)",
        hits.len(),
        elapsed.as_secs_f64() * 1000.0,
        index.document_count()
    );
    for (rank, hit) in hits.iter().enumerate() {
        println!("{:>3}. {:.3}  {}", rank + 1, hit.score, hit.url);
        if !hit.title.is_empty() {
            println!("      {}", hit.title);
        }
    }
    Ok(())
}

fn stats(options: &Options) -> Result<(), String> {
    let index = LocalIndex::open(&options.index)?;
    println!("documents: {}", index.document_count());
    Ok(())
}

/// The title of a page, falling back to the first heading.
fn page_title(html: &Html) -> String {
    for selector in ["title", "h1"] {
        if let Ok(parsed) = Selector::parse(selector) {
            if let Some(element) = html.select(&parsed).next() {
                let text = element.text().collect::<Vec<_>>().join(" ");
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !text.is_empty() {
                    return text;
                }
            }
        }
    }
    String::new()
}

/// Absolute, normalised http(s) links of the page that may be followed.
fn links(html: &Html, base: &url::Url) -> Vec<String> {
    let Ok(selector) = Selector::parse("a[href]") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for element in html.select(&selector) {
        if crawl::rel_forbids_following(element.value().attr("rel").unwrap_or("")) {
            continue;
        }
        let Some(href) = element.value().attr("href") else {
            continue;
        };
        let Ok(resolved) = base.join(href) else {
            continue;
        };
        let path = resolved.path().to_lowercase();
        if SKIP_EXTENSIONS
            .iter()
            .any(|extension| path.ends_with(extension))
        {
            continue;
        }
        if let Some(clean) = crawl::normalize_url(resolved.as_str()) {
            found.push(clean);
        }
    }
    found
}

/// `host` or `host:port`: the unit robots.txt and politeness apply to.
fn authority(url: &url::Url) -> Option<String> {
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

/// `www.example.com` and `example.com` count as the same site.
fn bare_host(host: &str) -> &str {
    host.strip_prefix("www.").unwrap_or(host)
}

/// A blocking client that identifies itself and only follows redirects that
/// stay on the same site (robots.txt was checked for that site only).
fn build_client(contact: &str) -> Result<reqwest::blocking::Client, String> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        let same_site = attempt.previous().last().is_none_or(|previous| {
            previous.host_str().map(bare_host) == attempt.url().host_str().map(bare_host)
        });
        if attempt.previous().len() >= MAX_REDIRECTS {
            attempt.error("too many redirects")
        } else if same_site {
            attempt.follow()
        } else {
            attempt.stop()
        }
    });
    reqwest::blocking::Client::builder()
        .user_agent(crawl::user_agent(contact))
        .redirect(policy)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("client: {error}"))
}

/// Read at most `limit` bytes of a response body.
fn read_capped(response: reqwest::blocking::Response, limit: usize) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    response
        .take(limit as u64 + 1)
        .read_to_end(&mut body)
        .ok()?;
    (body.len() <= limit).then_some(body)
}

/// What a worker hands back to the writer.
struct Fetched {
    /// The canonical address the page is indexed under.
    url: String,
    title: String,
    body: String,
    lang: String,
    links: Vec<String>,
    depth: usize,
}

/// The outcome of asking for one page.
enum Fetch {
    Page(Fetched),
    /// Not indexed, but its links may still be worth following.
    Links(Vec<String>),
    /// Moved to another spelling of the same site: crawl that address instead.
    Moved(String),
    /// The host asked us to slow down.
    Backoff(Duration),
    Skipped,
}

/// Counters printed at the end of a crawl.
#[derive(Default)]
struct Counters {
    robots_blocked: AtomicUsize,
    noindex: AtomicUsize,
    duplicates: AtomicUsize,
    failed: AtomicUsize,
    backoffs: AtomicUsize,
    host_capped: AtomicUsize,
}

/// The `host[:port]` a URL belongs to.
fn host_key(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| authority(&parsed))
        .unwrap_or_default()
}

/// Pending URLs grouped by host.
///
/// One FIFO queue lets a crowded host (a documentation site with thousands of
/// links) fill the front, so every worker ends up waiting for that host's
/// politeness delay while the other hosts starve. Grouping by host and always
/// serving a host that is ready keeps all workers busy.
#[derive(Default)]
struct Frontier {
    hosts: HashMap<String, VecDeque<(String, usize)>>,
    /// Hosts with pending URLs, in round-robin order.
    order: VecDeque<String>,
    len: usize,
}

impl Frontier {
    fn push(&mut self, url: String, depth: usize) {
        let host = host_key(&url);
        let pending = self.hosts.entry(host.clone()).or_default();
        if pending.is_empty() {
            self.order.push_back(host);
        }
        pending.push_back((url, depth));
        self.len += 1;
    }

    /// Take a URL of the first host whose next slot has come; when every host
    /// is still cooling down, the one that becomes ready first.
    fn pop(&mut self, slots: &HashMap<String, Instant>) -> Option<(String, usize)> {
        let now = Instant::now();
        let position = self
            .order
            .iter()
            .position(|host| slots.get(host).is_none_or(|slot| *slot <= now))
            .or_else(|| {
                (0..self.order.len()).min_by_key(|index| slots.get(&self.order[*index]).copied())
            })?;
        let host = self.order.remove(position)?;
        let pending = self.hosts.get_mut(&host)?;
        let item = pending.pop_front();
        if pending.is_empty() {
            self.hosts.remove(&host);
        } else {
            self.order.push_back(host);
        }
        self.len -= usize::from(item.is_some());
        item
    }
}

/// Shared crawl state. Workers pull URLs, the main thread writes documents.
struct Crawl {
    queue: Mutex<Frontier>,
    seen: Mutex<HashSet<String>>,
    robots: Mutex<HashMap<String, Arc<Robots>>>,
    /// Earliest time the next request to a host may start.
    next_slot: Mutex<HashMap<String, Instant>>,
    /// Pages claimed per host, against `max_per_host`.
    per_host: Mutex<HashMap<String, usize>>,
    in_flight: AtomicUsize,
    counters: Counters,
    /// With `--within-seed-path`: the path prefixes each host may be crawled
    /// under (the directories of its seeds). `None` crawls whole hosts.
    scope: Option<HashMap<String, Vec<String>>>,
    /// URLs taken from sitemaps: the only ones `--skip-indexed` may skip,
    /// because their links are not needed to discover the rest of the site.
    from_sitemap: Mutex<HashSet<String>>,
}

impl Crawl {
    fn new() -> Self {
        Self {
            queue: Mutex::new(Frontier::default()),
            seen: Mutex::new(HashSet::new()),
            robots: Mutex::new(HashMap::new()),
            next_slot: Mutex::new(HashMap::new()),
            per_host: Mutex::new(HashMap::new()),
            in_flight: AtomicUsize::new(0),
            counters: Counters::default(),
            scope: None,
            from_sitemap: Mutex::new(HashSet::new()),
        }
    }

    /// Give back a per-host slot claimed for a page that was not fetched.
    fn refund_host(&self, url: &str) {
        if let Ok(mut counts) = self.per_host.lock() {
            if let Some(count) = counts.get_mut(&host_key(url)) {
                *count = count.saturating_sub(1);
            }
        }
    }

    /// Whether `url` lies under one of its host's seed directories.
    fn in_scope(&self, url: &str) -> bool {
        let Some(scope) = &self.scope else {
            return true;
        };
        let Ok(parsed) = url::Url::parse(url) else {
            return false;
        };
        scope.get(&host_key(url)).is_some_and(|prefixes| {
            prefixes
                .iter()
                .any(|prefix| parsed.path().starts_with(prefix.as_str()))
        })
    }

    fn claim(&self, max_per_host: usize) -> Option<(String, usize)> {
        let mut queue = self.queue.lock().ok()?;
        loop {
            let popped = {
                let slots = self.next_slot.lock().ok()?;
                queue.pop(&slots)
            };
            let (url, depth) = popped?;
            let fresh = self
                .seen
                .lock()
                .map(|mut seen| seen.insert(url.clone()))
                .unwrap_or(false);
            if !fresh {
                continue;
            }
            let within_cap = self
                .per_host
                .lock()
                .map(|mut counts| {
                    let count = counts.entry(host_key(&url)).or_insert(0);
                    *count += 1;
                    *count <= max_per_host
                })
                .unwrap_or(true);
            if within_cap {
                return Some((url, depth));
            }
            self.counters.host_capped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Wait until this host may be asked again, so several workers still keep
    /// one request per delay per host.
    fn wait_for_host(&self, host: &str, delay: Duration) {
        loop {
            let slot = {
                let Ok(mut slots) = self.next_slot.lock() else {
                    return;
                };
                let now = Instant::now();
                let slot = slots.entry(host.to_string()).or_insert(now);
                if *slot <= now {
                    *slot = now + delay;
                    return;
                }
                *slot
            };
            std::thread::sleep(slot.saturating_duration_since(Instant::now()));
        }
    }

    /// Keep every worker away from this host for a while.
    fn back_off(&self, host: &str, pause: Duration) {
        if let Ok(mut slots) = self.next_slot.lock() {
            let until = Instant::now() + pause;
            let slot = slots.entry(host.to_string()).or_insert(until);
            if *slot < until {
                *slot = until;
            }
        }
    }

    /// The robots.txt policy of a host, fetched once.
    fn robots_for(
        &self,
        client: &reqwest::blocking::Client,
        scheme: &str,
        host: &str,
        delay: Duration,
    ) -> Arc<Robots> {
        if let Some(cached) = self
            .robots
            .lock()
            .ok()
            .and_then(|robots| robots.get(host).cloned())
        {
            return cached;
        }
        self.wait_for_host(host, delay);
        let robots_url = format!("{scheme}://{host}/robots.txt");
        let policy = match client.get(&robots_url).send() {
            Ok(response) => {
                let status = response.status().as_u16();
                let body = read_capped(response, 512 * 1024)
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    .unwrap_or_default();
                Robots::from_response(status, &body, crawl::PRODUCT)
            }
            // Unreachable robots.txt: crawling blind is not polite.
            Err(_) => Robots::deny_all(),
        };
        let policy = Arc::new(policy);
        if let Ok(mut robots) = self.robots.lock() {
            robots.insert(host.to_string(), Arc::clone(&policy));
        }
        policy
    }
}

/// URLs listed in a sitemap (following a sitemap index up to two levels).
/// Only entries on the sitemap's own host are accepted.
fn sitemap_urls(
    client: &reqwest::blocking::Client,
    url: &str,
    depth: usize,
    limit: usize,
    keep: &dyn Fn(&str) -> bool,
    out: &mut Vec<String>,
) {
    if depth > 2 || out.len() >= limit {
        return;
    }
    let Some(host) = url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
    else {
        return;
    };
    let Ok(response) = client.get(url).send() else {
        return;
    };
    if !response.status().is_success() {
        return;
    }
    let Some(bytes) = read_capped(response, 10 * 1024 * 1024) else {
        return;
    };
    let body = String::from_utf8_lossy(&bytes).into_owned();
    let is_index = body.contains("<sitemapindex");
    let mut found = 0usize;
    for entry in body.match_indices("<loc>") {
        let rest = &body[entry.0 + 5..];
        let Some(end) = rest.find("</loc>") else {
            continue;
        };
        let raw = rest[..end].trim();
        let loc = raw
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">");
        let Some(loc) = crawl::normalize_url(&loc) else {
            continue;
        };
        let same_host = url::Url::parse(&loc)
            .ok()
            .and_then(|parsed| parsed.host_str().map(|h| bare_host(h) == bare_host(&host)))
            .unwrap_or(false);
        if !same_host {
            continue;
        }
        found += 1;
        if is_index {
            sitemap_urls(client, &loc, depth + 1, limit, keep, out);
        } else if keep(&loc) {
            out.push(loc);
        }
        if out.len() >= limit || found > 5_000 {
            return;
        }
    }
}

/// The directory of a seed URL's path: `/3/` for `/3/`, `/doc/` for
/// `/doc/index.html`, `/` for `/docs.html`.
fn seed_directory(url: &str) -> String {
    let path = url::Url::parse(url)
        .map(|parsed| parsed.path().to_string())
        .unwrap_or_else(|_| "/".to_string());
    match path.rfind('/') {
        Some(end) => path[..=end].to_string(),
        None => "/".to_string(),
    }
}

fn crawl(options: &Options) -> Result<(), String> {
    let seeds_path = options.seeds.as_ref().ok_or("crawl needs --seeds <file>")?;
    let seeds = std::fs::read_to_string(seeds_path)
        .map_err(|error| format!("read {seeds_path:?}: {error}"))?;
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    for line in seeds.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(url) = crawl::normalize_url(line) {
            queue.push_back((url, 0));
        }
    }
    if queue.is_empty() {
        return Err("no seeds".to_string());
    }
    let delay = Duration::from_millis(options.delay_ms);
    let mut state = Crawl::new();
    if options.within_seed_path {
        let mut scope: HashMap<String, Vec<String>> = HashMap::new();
        for (url, _) in &queue {
            scope
                .entry(host_key(url))
                .or_default()
                .push(seed_directory(url));
        }
        state.scope = Some(scope);
    }
    let state = Arc::new(state);

    // Site roots advertise their sitemaps in robots.txt. Following them is
    // far more thorough than walking links, and it is what lets the index
    // cover a documentation site completely.
    if options.use_sitemaps {
        let client = build_client(&options.contact)?;
        let mut expanded: Vec<String> = Vec::new();
        let mut asked: HashSet<String> = HashSet::new();
        for (url, _) in queue.iter() {
            let Ok(parsed) = url::Url::parse(url) else {
                continue;
            };
            let Some(host) = authority(&parsed) else {
                continue;
            };
            if !asked.insert(host.clone()) {
                continue;
            }
            let robots = state.robots_for(&client, parsed.scheme(), &host, delay);
            // Each host gets its own share, so one huge sitemap cannot use up
            // the whole crawl before the other seeds are even looked at.
            let mut from_host: Vec<String> = Vec::new();
            for sitemap in robots.sitemaps() {
                state.wait_for_host(&host, delay);
                sitemap_urls(
                    &client,
                    sitemap,
                    0,
                    options.max_per_host,
                    &|url| state.in_scope(url),
                    &mut from_host,
                );
            }
            println!("sitemaps of {host}: {} urls", from_host.len());
            expanded.extend(from_host);
            if expanded.len() >= options.max_pages {
                break;
            }
        }
        if !expanded.is_empty() {
            println!("sitemaps added {} urls", expanded.len());
            let _ = std::io::stdout().flush();
            if let Ok(mut listed) = state.from_sitemap.lock() {
                listed.extend(expanded.iter().cloned());
            }
            for url in expanded {
                queue.push_back((url, 0));
            }
        }
    }
    if let Ok(mut shared) = state.queue.lock() {
        for (url, depth) in queue {
            shared.push(url, depth);
        }
    }

    let index = LocalIndex::open_for_write(&options.index)?;
    let crawl = state;
    let (sender, receiver) = std::sync::mpsc::channel::<Fetched>();
    let max_depth = options.max_depth;
    let max_per_host = options.max_per_host;
    // `--skip-indexed`: a resumed crawl does not download pages it already
    // holds (their links are then not followed; sitemaps cover that).
    let known = if options.skip_indexed {
        Some(Arc::new(LocalIndex::open(&options.index)?))
    } else {
        None
    };

    for _ in 0..options.workers {
        let crawl = Arc::clone(&crawl);
        let known = known.clone();
        let sender = sender.clone();
        let contact = options.contact.clone();
        std::thread::spawn(move || {
            let Ok(client) = build_client(&contact) else {
                return;
            };
            loop {
                let Some((url, depth)) = crawl.claim(max_per_host) else {
                    if crawl.in_flight.load(Ordering::SeqCst) == 0 {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                };
                let listed = crawl
                    .from_sitemap
                    .lock()
                    .map(|listed| listed.contains(&url))
                    .unwrap_or(false);
                if listed && known.as_ref().is_some_and(|index| index.has_url(&url)) {
                    crawl.refund_host(&url);
                    continue;
                }
                let Ok(parsed) = url::Url::parse(&url) else {
                    continue;
                };
                let Some(host) = authority(&parsed) else {
                    continue;
                };
                // in_flight covers the whole attempt: a worker that is still
                // fetching may queue more links, so the others must not quit.
                crawl.in_flight.fetch_add(1, Ordering::SeqCst);
                let robots = crawl.robots_for(&client, parsed.scheme(), &host, delay);
                let target = match parsed.query() {
                    Some(query) => format!("{}?{query}", parsed.path()),
                    None => parsed.path().to_string(),
                };
                if !robots.allowed(&target) {
                    crawl
                        .counters
                        .robots_blocked
                        .fetch_add(1, Ordering::Relaxed);
                    crawl.in_flight.fetch_sub(1, Ordering::SeqCst);
                    continue;
                }
                // The host's own Crawl-delay wins when it is longer than ours.
                let host_delay = robots.crawl_delay().map_or(delay, |own| own.max(delay));
                crawl.wait_for_host(&host, host_delay);
                let outcome = fetch(&client, &url, depth, max_depth, &crawl.counters);
                let mut to_send = None;
                match outcome {
                    Fetch::Page(mut page) => {
                        queue_links(&crawl, std::mem::take(&mut page.links), page.depth + 1);
                        to_send = Some(page);
                    }
                    Fetch::Links(found) => queue_links(&crawl, found, depth + 1),
                    Fetch::Moved(target) => queue_links(&crawl, vec![target], depth),
                    Fetch::Backoff(pause) => {
                        crawl.counters.backoffs.fetch_add(1, Ordering::Relaxed);
                        crawl.back_off(&host, pause);
                    }
                    Fetch::Skipped => {}
                }
                crawl.in_flight.fetch_sub(1, Ordering::SeqCst);
                if let Some(page) = to_send {
                    if sender.send(page).is_err() {
                        return;
                    }
                }
            }
        });
    }
    drop(sender);

    let mut indexed = 0usize;
    let mut fingerprints: HashSet<u64> = HashSet::new();
    for page in receiver {
        if indexed >= options.max_pages {
            break;
        }
        // The same text under another URL is one page.
        if !fingerprints.insert(crawl::content_fingerprint(&page.body)) {
            crawl.counters.duplicates.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        if index.has_url(&page.url) {
            continue;
        }
        index.add(&Page {
            url: &page.url,
            title: &page.title,
            body: &page.body,
            lang: &page.lang,
            snippet: "",
        })?;
        indexed += 1;
        if indexed % 50 == 0 {
            index.commit()?;
            let queued = crawl.queue.lock().map(|q| q.len).unwrap_or(0);
            println!("indexed {indexed} pages (queue {queued})");
            let _ = std::io::stdout().flush();
        }
    }

    index.commit()?;
    let counters = &crawl.counters;
    println!(
        "crawled {indexed} pages, index now has {} documents",
        index.document_count()
    );
    println!(
        "skipped: {} by robots.txt, {} noindex, {} duplicate text, {} over the per-host cap; {} failed, {} back-offs",
        counters.robots_blocked.load(Ordering::Relaxed),
        counters.noindex.load(Ordering::Relaxed),
        counters.duplicates.load(Ordering::Relaxed),
        counters.host_capped.load(Ordering::Relaxed),
        counters.failed.load(Ordering::Relaxed),
        counters.backoffs.load(Ordering::Relaxed),
    );
    Ok(())
}

fn queue_links(crawl: &Crawl, links: Vec<String>, depth: usize) {
    if let Ok(mut queue) = crawl.queue.lock() {
        for link in links {
            if crawl.in_scope(&link) {
                queue.push(link, depth);
            }
        }
    }
}

/// How long to stay away after a 429/503, from `Retry-After` when it is given
/// in seconds.
fn retry_after(response: &reqwest::blocking::Response) -> Duration {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_BACKOFF)
        .min(MAX_BACKOFF)
}

/// The language of a page: its `lang` attribute, else a script heuristic.
fn page_language(html: &Html, title: &str, body: &str) -> String {
    if let Ok(selector) = Selector::parse("html[lang]") {
        if let Some(lang) = html
            .select(&selector)
            .next()
            .and_then(|element| element.value().attr("lang"))
        {
            let primary = lang.split(['-', '_']).next().unwrap_or("").to_lowercase();
            if (2..=3).contains(&primary.len()) {
                return primary;
            }
        }
    }
    if has_cjk(title) || has_cjk(body) {
        "ja".to_string()
    } else {
        "en".to_string()
    }
}

/// Fetch one page and extract what the index needs.
fn fetch(
    client: &reqwest::blocking::Client,
    url: &str,
    depth: usize,
    max_depth: usize,
    counters: &Counters,
) -> Fetch {
    let Ok(response) = client.get(url).send() else {
        counters.failed.fetch_add(1, Ordering::Relaxed);
        return Fetch::Skipped;
    };
    let status = response.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
    {
        return Fetch::Backoff(retry_after(&response));
    }
    // A redirect we refused to follow (another site) or a plain failure.
    if !status.is_success() {
        counters.failed.fetch_add(1, Ordering::Relaxed);
        return Fetch::Skipped;
    }
    let (Ok(base), Ok(requested)) = (
        url::Url::parse(url),
        url::Url::parse(response.url().as_str()),
    ) else {
        return Fetch::Skipped;
    };
    // A www/non-www hop lands on a host whose robots.txt we have not read:
    // queue the final address and let it go through the normal checks.
    if requested.host_str() != base.host_str() {
        return match crawl::normalize_url(requested.as_str()) {
            Some(moved) => Fetch::Moved(moved),
            None => Fetch::Skipped,
        };
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !content_type.contains("text/html") {
        return Fetch::Skipped;
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY_BYTES as u64)
    {
        counters.failed.fetch_add(1, Ordering::Relaxed);
        return Fetch::Skipped;
    }
    let mut directives = response
        .headers()
        .get_all("x-robots-tag")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(crawl::header_directives)
        .fold(Directives::default(), |all, one| Directives {
            noindex: all.noindex || one.noindex,
            nofollow: all.nofollow || one.nofollow,
        });
    let Some(bytes) = read_capped(response, MAX_BODY_BYTES) else {
        counters.failed.fetch_add(1, Ordering::Relaxed);
        return Fetch::Skipped;
    };
    let body = String::from_utf8_lossy(&bytes).into_owned();
    let html = Html::parse_document(&body);
    let meta = crawl::meta_directives(&html);
    directives.noindex |= meta.noindex;
    directives.nofollow |= meta.nofollow;

    let host = base.host_str().unwrap_or_default().to_string();
    let found = if depth < max_depth && !directives.nofollow {
        links(&html, &base)
            .into_iter()
            .filter(|link| {
                url::Url::parse(link)
                    .map(|parsed| parsed.host_str() == Some(host.as_str()))
                    .unwrap_or(false)
            })
            .collect()
    } else {
        Vec::new()
    };
    if directives.noindex {
        counters.noindex.fetch_add(1, Ordering::Relaxed);
        return Fetch::Links(found);
    }
    let title = page_title(&html);
    let text = rustxng::content::main_text_limited(&html, 20_000);
    if title.is_empty() && text.is_empty() {
        return Fetch::Links(found);
    }
    let canonical = crawl::canonical_url(&html, &base)
        .or_else(|| crawl::normalize_url(url))
        .unwrap_or_else(|| url.to_string());
    let lang = page_language(&html, &title, &text);
    Fetch::Page(Fetched {
        url: canonical,
        title,
        body: text,
        lang,
        links: found,
        depth,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_scope_is_the_seed_directory() {
        assert_eq!(seed_directory("https://docs.python.org/3/"), "/3/");
        assert_eq!(seed_directory("https://www.sqlite.org/docs.html"), "/");
        assert_eq!(seed_directory("https://go.dev/doc/index.html"), "/doc/");
        let mut crawl = Crawl::new();
        crawl.scope = Some(HashMap::from([(
            host_key("https://docs.python.org/3/"),
            vec!["/3/".to_string()],
        )]));
        assert!(crawl.in_scope("https://docs.python.org/3/library/re.html"));
        assert!(!crawl.in_scope("https://docs.python.org/ja/3/library/re.html"));
        assert!(!crawl.in_scope("https://example.org/3/"));
    }

    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn crawl_follows_links_from_new_and_indexed_pages() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("http://{address}/");
        let directory = tempfile::tempdir().unwrap();
        let seeds = directory.path().join("seeds.txt");
        std::fs::write(&seeds, &url).unwrap();
        let mut cases = Vec::new();
        for name in ["fresh", "resumed"] {
            let mut options = Options::parse(&[]);
            options.seeds = Some(seeds.clone());
            options.index = directory.path().join(name);
            options.workers = 1;
            options.delay_ms = 0;
            options.max_pages = 3;
            options.max_depth = 2;
            if name == "resumed" {
                let index = LocalIndex::open_for_write(&options.index).unwrap();
                index
                    .add(&Page {
                        url: &url,
                        title: "Root",
                        body: "Child",
                        lang: "en",
                        snippet: "",
                    })
                    .unwrap();
                index.commit().unwrap();
            }
            cases.push(options);
        }

        let server = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(&stream);
                let mut request = String::new();
                if reader.read_line(&mut request).unwrap() == 0 {
                    break;
                }
                for header in reader.lines() {
                    if header.unwrap().is_empty() {
                        break;
                    }
                }
                let body = match request.split_whitespace().nth(1).unwrap() {
                    "/" => "<title>Root</title><a href='/child'>Child</a>",
                    "/child" => "<title>Child</title><a href='/grandchild'>Grandchild</a>",
                    "/grandchild" => "<title>Grandchild</title><p>The last page</p>",
                    _ => "",
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let results: Vec<_> = cases
            .iter()
            .map(|options| {
                crawl(options)?;
                LocalIndex::open(&options.index).map(|index| index.document_count())
            })
            .collect();
        drop(TcpStream::connect(address).unwrap());
        server.join().unwrap();
        assert_eq!(results, vec![Ok(3), Ok(3)]);
    }

    #[test]
    fn crawl_obeys_robots_noindex_nofollow_and_announces_itself() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let seeds = directory.path().join("seeds.txt");
        std::fs::write(&seeds, format!("http://{address}/")).unwrap();
        let mut options = Options::parse(&[]);
        options.seeds = Some(seeds);
        options.index = directory.path().join("index");
        options.workers = 1;
        options.delay_ms = 0;
        options.max_pages = 20;
        options.max_depth = 3;
        options.contact = "ops@example.org".into();

        let requests: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let log = Arc::clone(&requests);
        let server = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(&stream);
                let mut request = String::new();
                if reader.read_line(&mut request).unwrap() == 0 {
                    break;
                }
                let path = request.split_whitespace().nth(1).unwrap().to_string();
                let mut agent = String::new();
                for header in reader.lines() {
                    let header = header.unwrap();
                    if header.is_empty() {
                        break;
                    }
                    if let Some(value) = header.to_lowercase().strip_prefix("user-agent:") {
                        agent = value.trim().to_string();
                    }
                }
                log.lock().unwrap().push((path.clone(), agent));
                let (kind, extra, body) = match path.as_str() {
                    "/robots.txt" => (
                        "text/plain",
                        "",
                        "User-agent: *\nDisallow: /private\n".to_string(),
                    ),
                    "/" => (
                        "text/html",
                        "",
                        "<title>Root</title><a href='/ok'>ok</a><a href='/private/secret'>p</a>\
                         <a href='/hidden'>h</a><a rel='nofollow' href='/nofollowed'>n</a>\
                         <a href='/header-noindex'>x</a>"
                            .to_string(),
                    ),
                    "/ok" => (
                        "text/html",
                        "",
                        "<title>Fine</title><p>fine page</p>".to_string(),
                    ),
                    "/hidden" => (
                        "text/html",
                        "",
                        "<meta name='robots' content='noindex'><title>Hidden</title>\
                         <a href='/after-hidden'>a</a>"
                            .to_string(),
                    ),
                    "/after-hidden" => (
                        "text/html",
                        "",
                        "<title>After</title><p>reachable through a noindex page</p>".to_string(),
                    ),
                    "/header-noindex" => (
                        "text/html",
                        "X-Robots-Tag: noindex\r\n",
                        "<title>Header</title><p>never indexed</p>".to_string(),
                    ),
                    _ => ("text/html", "", String::new()),
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });

        crawl(&options).unwrap();
        drop(TcpStream::connect(address).unwrap());
        server.join().unwrap();

        let index = LocalIndex::open(&options.index).unwrap();
        let mut found: Vec<String> = ["Root", "Fine", "After", "Hidden", "Header", "never"]
            .iter()
            .flat_map(|word| index.search(word, 5).unwrap())
            .map(|hit| hit.title)
            .collect();
        found.sort();
        found.dedup();
        assert_eq!(found, vec!["After", "Fine", "Root"], "indexed titles");

        let requests = requests.lock().unwrap();
        let paths: Vec<&str> = requests.iter().map(|(path, _)| path.as_str()).collect();
        assert!(
            !paths.contains(&"/private/secret"),
            "robots.txt ignored: {paths:?}"
        );
        assert!(
            !paths.contains(&"/nofollowed"),
            "rel=nofollow ignored: {paths:?}"
        );
        assert!(
            paths.contains(&"/after-hidden"),
            "links of a noindex page are followed"
        );
        for (path, agent) in requests.iter() {
            assert!(
                agent.starts_with("rustxng-index/") && agent.contains("ops@example.org"),
                "{path} was fetched as {agent:?}"
            );
        }
    }

    #[test]
    fn a_ready_host_is_served_before_a_cooling_crowded_one() {
        let mut frontier = Frontier::default();
        for page in 0..5 {
            frontier.push(format!("https://busy.example/{page}"), 0);
        }
        frontier.push("https://quiet.example/".to_string(), 0);
        assert_eq!(frontier.len, 6);
        let mut slots = HashMap::new();
        slots.insert(
            "busy.example".to_string(),
            Instant::now() + Duration::from_secs(60),
        );
        let (url, _) = frontier.pop(&slots).unwrap();
        assert_eq!(url, "https://quiet.example/");
        // Only the cooling host is left: it is still served, earliest first.
        let (url, _) = frontier.pop(&slots).unwrap();
        assert_eq!(url, "https://busy.example/0");
        assert_eq!(frontier.len, 4);
    }

    #[test]
    fn ready_hosts_take_turns() {
        let mut frontier = Frontier::default();
        for page in 0..2 {
            frontier.push(format!("https://a.example/{page}"), 0);
            frontier.push(format!("https://b.example/{page}"), 0);
        }
        let slots = HashMap::new();
        let order: Vec<String> = (0..4).map(|_| frontier.pop(&slots).unwrap().0).collect();
        assert_eq!(
            order,
            [
                "https://a.example/0",
                "https://b.example/0",
                "https://a.example/1",
                "https://b.example/1"
            ]
        );
        assert!(frontier.pop(&slots).is_none());
        assert_eq!(frontier.len, 0);
    }
}
