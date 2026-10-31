//! Crawl a seed list into the local index, and query it from the terminal.
//!
//!   rustxng-index crawl --seeds seeds.txt --out ./index-data --max-pages 2000
//!   rustxng-index search --index ./index-data "tokio runtime"
//!   rustxng-index stats --index ./index-data

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::time::Duration;

use rustxng::localindex::{LocalIndex, Page, has_cjk};
use scraper::{Html, Selector};

/// Skip files that are not pages.
const SKIP_EXTENSIONS: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".ico", ".css", ".js", ".json", ".zip",
    ".gz", ".tar", ".pdf", ".mp4", ".mp3", ".woff", ".woff2", ".ttf", ".xml", ".rss", ".txt",
];

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
    delay_ms: u64,
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
            delay_ms: 500,
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
                    if let Some(value) = next().and_then(|v| v.parse().ok()) {
                        options.max_pages = value;
                    }
                }
                "--max-depth" => {
                    if let Some(value) = next().and_then(|v| v.parse().ok()) {
                        options.max_depth = value;
                    }
                }
                "--delay-ms" => {
                    if let Some(value) = next().and_then(|v| v.parse().ok()) {
                        options.delay_ms = value;
                    }
                }
                "--limit" => {
                    if let Some(value) = next().and_then(|v| v.parse().ok()) {
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
    let started = std::time::Instant::now();
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

/// Absolute http(s) links found on the page.
fn links(html: &Html, base: &url::Url) -> Vec<String> {
    let Ok(selector) = Selector::parse("a[href]") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for element in html.select(&selector) {
        let Some(href) = element.value().attr("href") else {
            continue;
        };
        let Ok(resolved) = base.join(href) else {
            continue;
        };
        if !matches!(resolved.scheme(), "http" | "https") {
            continue;
        }
        let path = resolved.path().to_lowercase();
        if SKIP_EXTENSIONS
            .iter()
            .any(|extension| path.ends_with(extension))
        {
            continue;
        }
        let mut clean = resolved.clone();
        clean.set_fragment(None);
        found.push(clean.to_string());
    }
    found
}

/// The Disallow rules that apply to everyone.
fn disallowed_paths(robots: &str) -> Vec<String> {
    let mut rules = Vec::new();
    let mut applies = false;
    for line in robots.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.len() > 12 && line[..12].eq_ignore_ascii_case("user-agent: ") {
            let agent = line[12..].trim();
            applies = agent == "*";
        } else if applies {
            if let Some(rest) = line.strip_prefix("Disallow:") {
                let path = rest.trim();
                if !path.is_empty() {
                    rules.push(path.to_string());
                }
            }
        }
    }
    rules
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
        queue.push_back((line.to_string(), 0));
    }
    if queue.is_empty() {
        return Err("no seeds".to_string());
    }

    let index = LocalIndex::open_for_write(&options.index)?;
    let client = reqwest::blocking::Client::builder()
        .user_agent("rustxng-index/0.1 (+https://github.com/sahenjp/RustXNG)")
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("client: {error}"))?;

    let mut seen: HashSet<String> = HashSet::new();
    let mut robots: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut indexed = 0usize;

    while let Some((url, depth)) = queue.pop_front() {
        if indexed >= options.max_pages {
            break;
        }
        if !seen.insert(url.clone()) {
            continue;
        }
        let Ok(parsed) = url::Url::parse(&url) else {
            continue;
        };
        let Some(host) = parsed.host_str().map(str::to_string) else {
            continue;
        };

        // Respect robots.txt once per host.
        let rules = robots.entry(host.clone()).or_insert_with(|| {
            let robots_url = format!("{}://{host}/robots.txt", parsed.scheme());
            client
                .get(&robots_url)
                .send()
                .ok()
                .and_then(|response| response.text().ok())
                .map(|body| disallowed_paths(&body))
                .unwrap_or_default()
        });
        if rules
            .iter()
            .any(|rule| parsed.path().starts_with(rule.trim_end_matches('*')))
        {
            continue;
        }

        let response = match client.get(&url).send() {
            Ok(response) => response,
            Err(_) => continue,
        };
        if !response.status().is_success() {
            continue;
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !content_type.contains("text/html") {
            continue;
        }
        let body = match response.text() {
            Ok(body) => body,
            Err(_) => continue,
        };
        let html = Html::parse_document(&body);
        let title = page_title(&html);
        let text = rustxng::content::main_text_limited(&html, 20_000);
        if title.is_empty() && text.is_empty() {
            continue;
        }
        let lang = if has_cjk(&title) || has_cjk(&text) {
            "ja"
        } else {
            "en"
        };
        index.add(&Page {
            url: &url,
            title: &title,
            body: &text,
            lang,
        })?;
        indexed += 1;
        // Commit periodically: a long crawl that dies halfway should keep what
        // it already fetched.
        if indexed % 50 == 0 {
            index.commit()?;
            println!("indexed {indexed} pages (queue {})", queue.len());
        }

        if depth < options.max_depth {
            for link in links(&html, &parsed) {
                if seen.contains(&link) {
                    continue;
                }
                let Ok(link_url) = url::Url::parse(&link) else {
                    continue;
                };
                if link_url.host_str() != Some(host.as_str()) {
                    continue;
                }
                queue.push_back((link, depth + 1));
            }
        }
        std::thread::sleep(Duration::from_millis(options.delay_ms));
    }

    index.commit()?;
    println!(
        "crawled {indexed} pages, index now has {} documents",
        index.document_count()
    );
    Ok(())
}
