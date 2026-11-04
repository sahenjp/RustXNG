//! Build a local search index of Wikipedia from the official dump.
//!
//! The dump is streamed: `curl` downloads one part, `bzip2` unpacks it, and the
//! pages are indexed as they pass by. Nothing but the index is written to disk.
//! Wikimedia publishes the dumps for exactly this kind of reuse; the text is
//! CC BY-SA, so keep the attribution when showing results.
//!
//! ```text
//! rustxng-wikidump --index index-wikipedia \
//!     --base-url https://ftp.acc.umu.se/mirror/wikimedia.org/dumps/enwiki/20261001/ \
//!     --parallel 3
//! ```
//!
//! Needs `curl` and `bzip2` on the PATH. A run can be stopped and started
//! again: finished parts are listed in `<index>/parts-done.txt` and skipped.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rustxng::localindex::{LocalIndex, Page};
use rustxng::wikitext;

struct Options {
    index: PathBuf,
    base_url: String,
    parallel: usize,
    max_chars: usize,
    min_bytes: usize,
    limit_parts: usize,
    wiki: String,
}

fn usage() -> ! {
    eprintln!(
        "usage: rustxng-wikidump --index DIR --base-url URL [--parallel 3] [--max-chars 800]\n\
         \t[--min-bytes 1500] [--limit-parts N] [--wiki en]\n\n\
         BASE-URL is a dump directory such as\n\
         https://ftp.acc.umu.se/mirror/wikimedia.org/dumps/enwiki/20261001/"
    );
    std::process::exit(2);
}

fn parse_options() -> Options {
    let mut options = Options {
        index: PathBuf::from("index-wikipedia"),
        base_url: String::new(),
        parallel: 3,
        max_chars: 800,
        min_bytes: 1500,
        limit_parts: usize::MAX,
        wiki: "en".to_string(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| usage());
        match flag.as_str() {
            "--index" => options.index = PathBuf::from(value()),
            "--base-url" => options.base_url = value(),
            "--parallel" => options.parallel = value().parse().unwrap_or_else(|_| usage()),
            "--max-chars" => options.max_chars = value().parse().unwrap_or_else(|_| usage()),
            "--min-bytes" => options.min_bytes = value().parse().unwrap_or_else(|_| usage()),
            "--limit-parts" => options.limit_parts = value().parse().unwrap_or_else(|_| usage()),
            "--wiki" => options.wiki = value(),
            _ => usage(),
        }
    }
    if options.base_url.is_empty() {
        usage();
    }
    if !options.base_url.ends_with('/') {
        options.base_url.push('/');
    }
    options.parallel = options.parallel.clamp(1, 4);
    options
}

fn user_agent() -> String {
    rustxng::network::identified_user_agent("")
}

/// The multistream part files listed on a dump page, in listing order.
fn list_parts(base_url: &str) -> Result<Vec<String>, String> {
    let output = Command::new("curl")
        .args(["-sS", "--fail", "-A", &user_agent(), base_url])
        .output()
        .map_err(|error| format!("curl: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "listing {base_url} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let page = String::from_utf8_lossy(&output.stdout);
    let mut parts = Vec::new();
    let mut rest: &str = &page;
    while let Some(at) = rest.find("href=\"") {
        rest = &rest[at + 6..];
        let Some(end) = rest.find('"') else { break };
        let name = &rest[..end];
        if is_part_name(name) && !parts.iter().any(|known| known == name) {
            parts.push(name.to_string());
        }
        rest = &rest[end..];
    }
    Ok(parts)
}

/// `…-pages-articles-multistream12.xml-p123p456.bz2`, not the index files.
fn is_part_name(name: &str) -> bool {
    name.contains("pages-articles-multistream")
        && !name.contains("index")
        && name.ends_with(".bz2")
        && name.contains(".xml-p")
}

/// One article ready to be indexed.
struct Article {
    url: String,
    title: String,
    body: String,
    snippet: String,
}

/// Reads the dump XML line by line. MediaWiki exports are line-structured,
/// which makes a small scanner enough.
struct Scanner {
    wiki: String,
    max_chars: usize,
    min_bytes: usize,
    seen_pages: u64,
}

impl Scanner {
    /// Feed the whole stream; `sink` receives each article worth indexing.
    fn run(&mut self, input: impl BufRead, mut sink: impl FnMut(Article)) -> std::io::Result<()> {
        let mut reader = input;
        let mut buffer = Vec::new();
        let mut title = String::new();
        let mut namespace_ok = false;
        let mut redirect = false;
        let mut declared_bytes = 0usize;
        let mut in_text = false;
        let mut text = String::new();
        loop {
            buffer.clear();
            if reader.read_until(b'\n', &mut buffer)? == 0 {
                return Ok(());
            }
            let line = String::from_utf8_lossy(&buffer);
            let trimmed = line.trim_start();
            if in_text {
                if let Some(end) = line.find("</text>") {
                    push_capped(&mut text, &line[..end]);
                    in_text = false;
                } else {
                    push_capped(&mut text, &line);
                }
                continue;
            }
            if trimmed.starts_with("<page>") {
                title.clear();
                namespace_ok = false;
                redirect = false;
                declared_bytes = 0;
                text.clear();
            } else if let Some(rest) = trimmed.strip_prefix("<title>") {
                title = wikitext::decode_xml(rest.split("</title>").next().unwrap_or(""));
            } else if let Some(rest) = trimmed.strip_prefix("<ns>") {
                namespace_ok = rest.trim_start().starts_with("0</ns>");
            } else if trimmed.starts_with("<redirect") {
                redirect = true;
            } else if trimmed.starts_with("<text") {
                declared_bytes = attribute(trimmed, "bytes")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
                let Some(open_end) = trimmed.find('>') else {
                    continue;
                };
                if trimmed[..open_end].ends_with('/') {
                    continue;
                }
                let content = &trimmed[open_end + 1..];
                match content.find("</text>") {
                    Some(end) => push_capped(&mut text, &content[..end]),
                    None => {
                        push_capped(&mut text, content);
                        in_text = true;
                    }
                }
            } else if trimmed.starts_with("</page>") {
                self.seen_pages += 1;
                if namespace_ok && !redirect && declared_bytes >= self.min_bytes {
                    if let Some(article) = self.article(&title, &text) {
                        sink(article);
                    }
                }
            }
        }
    }

    fn article(&self, title: &str, wikitext_source: &str) -> Option<Article> {
        if title.is_empty() || title.ends_with("(disambiguation)") {
            return None;
        }
        let source = wikitext::decode_xml(wikitext_source);
        if source
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("#redirect")
        {
            return None;
        }
        let body = wikitext::lead(&source, self.max_chars);
        if body.chars().count() < 80 || body.contains(" may refer to") {
            return None;
        }
        let url = format!(
            "https://{}.wikipedia.org/wiki/{}",
            self.wiki,
            rustxng::engines::wikipedia::wiki_title_path(title)
        );
        let snippet = rustxng::crawl::snippet(&body, 280);
        Some(Article {
            url,
            title: title.to_string(),
            body,
            snippet,
        })
    }
}

/// Keep at most the first part of a very long article in memory.
fn push_capped(target: &mut String, piece: &str) {
    const CAP: usize = 200_000;
    if target.len() < CAP {
        target.push_str(piece);
    }
}

/// The value of `name="…"` in a tag.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!("{name}=\"");
    let start = tag.find(&marker)? + marker.len();
    let end = tag[start..].find('"')?;
    Some(&tag[start..start + end])
}

/// Index one part. Articles are held back until the whole part has been read,
/// so a failed download leaves nothing half-indexed.
fn index_part(
    options: &Options,
    index: &LocalIndex,
    name: &str,
    indexed: &AtomicU64,
) -> Result<u64, String> {
    let url = format!("{}{}", options.base_url, name);
    let mut curl = Command::new("curl")
        .args(["-sS", "--fail", "--retry", "3", "-A", &user_agent(), &url])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("curl: {error}"))?;
    let curl_out = curl.stdout.take().ok_or("curl produced no output")?;
    let mut bzip = Command::new("bzip2")
        .arg("-dc")
        .stdin(Stdio::from(curl_out))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("bzip2: {error}"))?;
    let unpacked = bzip.stdout.take().ok_or("bzip2 produced no output")?;
    let mut scanner = Scanner {
        wiki: options.wiki.clone(),
        max_chars: options.max_chars,
        min_bytes: options.min_bytes,
        seen_pages: 0,
    };
    let mut articles = Vec::new();
    let scanned = scanner.run(BufReader::with_capacity(1 << 20, unpacked), |article| {
        articles.push(article)
    });
    let bzip_status = bzip.wait().map_err(|error| error.to_string())?;
    let curl_status = curl.wait().map_err(|error| error.to_string())?;
    scanned.map_err(|error| format!("{name}: read: {error}"))?;
    if !curl_status.success() || !bzip_status.success() {
        return Err(format!(
            "{name}: download or unpack failed (curl {curl_status}, bzip2 {bzip_status})"
        ));
    }
    if scanner.seen_pages == 0 {
        return Err(format!("{name}: no pages found in the stream"));
    }
    let count = articles.len() as u64;
    for article in &articles {
        index.add(&Page {
            url: &article.url,
            title: &article.title,
            body: &article.body,
            lang: &options.wiki,
            snippet: &article.snippet,
        })?;
    }
    indexed.fetch_add(count, Ordering::Relaxed);
    Ok(count)
}

fn main() {
    let options = Arc::new(parse_options());
    let mut parts = match list_parts(&options.base_url) {
        Ok(parts) if !parts.is_empty() => parts,
        Ok(_) => {
            eprintln!("no multistream parts found at {}", options.base_url);
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    parts.truncate(options.limit_parts);
    let index = match LocalIndex::open_for_write(&options.index) {
        Ok(index) => Arc::new(index),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let done_file = options.index.join("parts-done.txt");
    let done: Vec<String> = std::fs::read_to_string(&done_file)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let queue: VecDeque<String> = parts
        .into_iter()
        .filter(|part| !done.contains(part))
        .collect();
    println!(
        "{} parts to index ({} already done), {} at a time",
        queue.len(),
        done.len(),
        options.parallel
    );
    let total = queue.len();
    let queue = Arc::new(Mutex::new(queue));
    let finished = Arc::new(AtomicU64::new(0));
    let indexed = Arc::new(AtomicU64::new(0));
    let failed = Arc::new(Mutex::new(Vec::<String>::new()));
    let started = Instant::now();
    let done_lock = Arc::new(Mutex::new(()));

    let workers: Vec<_> = (0..options.parallel)
        .map(|_| {
            let (options, index, queue, finished, indexed, failed, done_lock, done_file) = (
                Arc::clone(&options),
                Arc::clone(&index),
                Arc::clone(&queue),
                Arc::clone(&finished),
                Arc::clone(&indexed),
                Arc::clone(&failed),
                Arc::clone(&done_lock),
                done_file.clone(),
            );
            std::thread::spawn(move || {
                loop {
                    let Some(name) = queue.lock().ok().and_then(|mut queue| queue.pop_front())
                    else {
                        return;
                    };
                    let mut result = Err(String::new());
                    for attempt in 1..=3 {
                        result = index_part(&options, &index, &name, &indexed);
                        match &result {
                            Ok(_) => break,
                            Err(error) => eprintln!("attempt {attempt} failed: {error}"),
                        }
                    }
                    match result {
                        Ok(count) => {
                            if let Err(error) = index.commit() {
                                eprintln!("commit failed: {error}");
                            }
                            record_done(&done_file, &done_lock, &name);
                            let number = finished.fetch_add(1, Ordering::Relaxed) + 1;
                            println!(
                                "[{number}/{total}] {name}: {count} articles (total {}, {:.0} min)",
                                indexed.load(Ordering::Relaxed),
                                started.elapsed().as_secs_f64() / 60.0
                            );
                        }
                        Err(_) => {
                            if let Ok(mut failed) = failed.lock() {
                                failed.push(name);
                            }
                        }
                    }
                }
            })
        })
        .collect();
    for worker in workers {
        let _ = worker.join();
    }
    if let Err(error) = index.commit() {
        eprintln!("final commit failed: {error}");
    }
    println!(
        "indexed {} articles, the index now has {} documents",
        indexed.load(Ordering::Relaxed),
        index.document_count()
    );
    let failed = failed
        .lock()
        .map(|failed| failed.clone())
        .unwrap_or_default();
    if !failed.is_empty() {
        eprintln!(
            "{} parts failed; run the same command again to retry them: {}",
            failed.len(),
            failed.join(", ")
        );
        std::process::exit(1);
    }
}

fn record_done(path: &Path, lock: &Mutex<()>, name: &str) {
    let _guard = lock.lock();
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{name}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = "<mediawiki>\n  <page>\n    <title>Ada Lovelace</title>\n    <ns>0</ns>\n    <id>1</id>\n    <revision>\n      <text bytes=\"2000\" xml:space=\"preserve\">'''Ada Lovelace''' was an English mathematician and writer, chiefly known for her work on the Analytical Engine.&lt;ref&gt;x&lt;/ref&gt;\n== Life ==\nShe was born in London in 1815 and worked with Charles Babbage on early computing machines.</text>\n    </revision>\n  </page>\n  <page>\n    <title>Ada (redirect)</title>\n    <ns>0</ns>\n    <redirect title=\"Ada Lovelace\" />\n    <revision>\n      <text bytes=\"2000\" xml:space=\"preserve\">#REDIRECT [[Ada Lovelace]]</text>\n    </revision>\n  </page>\n  <page>\n    <title>Talk:Ada Lovelace</title>\n    <ns>1</ns>\n    <revision>\n      <text bytes=\"2000\" xml:space=\"preserve\">A discussion page with enough words to be long enough for the filter, definitely.</text>\n    </revision>\n  </page>\n  <page>\n    <title>Stub</title>\n    <ns>0</ns>\n    <revision>\n      <text bytes=\"300\" xml:space=\"preserve\">Tiny.</text>\n    </revision>\n  </page>\n</mediawiki>\n";

    #[test]
    fn only_real_articles_are_indexed() {
        let mut scanner = Scanner {
            wiki: "en".into(),
            max_chars: 200,
            min_bytes: 1500,
            seen_pages: 0,
        };
        let mut articles = Vec::new();
        scanner
            .run(DUMP.as_bytes(), |article| articles.push(article))
            .unwrap();
        assert_eq!(scanner.seen_pages, 4);
        assert_eq!(articles.len(), 1);
        assert_eq!(articles[0].title, "Ada Lovelace");
        assert_eq!(
            articles[0].url,
            "https://en.wikipedia.org/wiki/Ada_Lovelace"
        );
        assert!(
            articles[0]
                .body
                .starts_with("Ada Lovelace was an English mathematician")
        );
        assert!(!articles[0].body.contains("<ref>"));
    }

    #[test]
    fn part_names_exclude_index_files() {
        assert!(is_part_name(
            "enwiki-20261001-pages-articles-multistream1.xml-p1p41242.bz2"
        ));
        assert!(!is_part_name(
            "enwiki-20261001-pages-articles-multistream-index1.txt-p1p41242.bz2"
        ));
        assert!(!is_part_name(
            "enwiki-20261001-pages-articles-multistream.xml.bz2"
        ));
    }
}
