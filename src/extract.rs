//! Site map and structured extraction for agents (`/map` and `/extract`).
//!
//! `/map` answers "which pages does this site have" without crawling the whole
//! site: it reads `robots.txt` for sitemap declarations, follows sitemap
//! indexes, and falls back to the links on the seed page.
//!
//! `/extract` turns one page into records. The schema is resolved from the
//! page itself (JSON-LD, definition lists, tables) so the caller gets
//! structure instead of prose; a page without any of those yields one record
//! of reduced page text. Nothing is invented: every field carries the
//! selector or JSON-LD path it came from.

use std::collections::BTreeSet;

use scraper::{Html, Selector};

use crate::crawl::{normalize_url, sitemap_locations};
use crate::fetch::PageReader;
use crate::robots::Robots;

/// Most URLs one `/map` call returns.
pub const MAX_MAP_URLS: usize = 5_000;
/// Most sitemaps followed from one seed (index files expand to more).
pub const MAX_SITEMAPS: usize = 32;
/// Most links read from the seed page when no sitemap exists.
const MAX_SEED_LINKS: usize = 2_000;
/// Most records one `/extract` call returns.
pub const MAX_RECORDS: usize = 200;
/// Most characters of a field value.
const MAX_FIELD_CHARS: usize = 2_000;

/// Where one URL in a `/map` answer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UrlSource {
    /// Declared in `robots.txt`.
    RobotsSitemap,
    /// Found in a sitemap that was reached from `robots.txt`.
    SitemapIndex,
    /// A link on the seed page.
    SeedLinks,
}

/// One URL of a `/map` answer.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MappedUrl {
    pub url: String,
    /// 1 for a sitemap reached from `robots.txt`, 2 for one nested in an index.
    pub depth: u8,
    pub source: UrlSource,
}

/// The result of a `/map` call.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MapReport {
    /// The seed the caller asked about.
    pub seed: String,
    /// Origin the URLs were restricted to.
    pub origin: String,
    /// Whether a sitemap was used at all.
    pub used_sitemap: bool,
    /// Whether the list was cut at the limit.
    pub truncated: bool,
    /// URLs in discovery order, deduplicated and normalised.
    pub urls: Vec<MappedUrl>,
    /// Sitemaps that could not be read, with a short reason.
    pub errors: Vec<String>,
}

/// Read one body with the shared page reader.
///
/// The raw markup is required, not the extracted text: `robots.txt` and
/// sitemap declarations are line- and tag-sensitive, seed links live in
/// `<a href>` attributes, and `/extract` parses `<script>` and `<table>`
/// elements — all of which text extraction discards.
async fn body(reader: &PageReader, url: &str) -> Result<String, String> {
    reader.raw(url).await
}

/// Whether `candidate` belongs to `seed`'s origin (scheme, host and port).
///
/// A shared string prefix is not enough: `https://example.com` is a prefix of
/// `https://example.com.evil.example`, `https://example.computer` and
/// `https://example.com:8443`, none of which is the same origin.
fn same_origin(seed: &reqwest::Url, candidate: &str) -> bool {
    reqwest::Url::parse(candidate)
        .map(|url| url.origin() == seed.origin())
        .unwrap_or(false)
}

/// Collect the URLs of a site.
///
/// Discovery order: sitemaps named by `robots.txt`, then sitemap indexes
/// declared inside them, then (only when none of those produced a URL) the
/// links on the seed page. Every URL is normalised, restricted to the seed's
/// origin, and deduplicated.
pub async fn map_site(reader: &PageReader, seed: &str, limit: usize) -> Result<MapReport, String> {
    let limit = limit.clamp(1, MAX_MAP_URLS);
    let seed_url = reqwest::Url::parse(seed).map_err(|_| "not a valid URL".to_string())?;
    let origin = seed_url.origin().ascii_serialization();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut urls: Vec<MappedUrl> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut used_sitemap = false;

    let root = format!("{origin}/robots.txt");
    let mut queue: Vec<(String, u8, UrlSource)> = Vec::new();
    match body(reader, &root).await {
        Ok(text) => {
            let robots = Robots::parse(&text, crate::crawl::PRODUCT);
            for sitemap in robots.sitemaps().iter().take(MAX_SITEMAPS) {
                queue.push((sitemap.clone(), 1, UrlSource::RobotsSitemap));
            }
        }
        // A missing robots.txt is not an error: the seed links still work.
        Err(error) => errors.push(format!("robots.txt: {error}")),
    }

    let mut followed = 0usize;
    while let Some((sitemap, depth, source)) = queue.pop() {
        if followed >= MAX_SITEMAPS || urls.len() >= limit {
            break;
        }
        followed += 1;
        let text = match body(reader, &sitemap).await {
            Ok(text) => text,
            Err(error) => {
                errors.push(format!("{sitemap}: {error}"));
                continue;
            }
        };
        match sitemap_locations(text.as_bytes(), MAX_MAP_URLS) {
            Ok((is_index, locations)) => {
                used_sitemap = true;
                if is_index {
                    for location in locations {
                        if queue.len() >= MAX_SITEMAPS {
                            break;
                        }
                        queue.push((location, depth.saturating_add(1), UrlSource::SitemapIndex));
                    }
                    continue;
                }
                for location in locations {
                    if urls.len() >= limit {
                        break;
                    }
                    if !same_origin(&seed_url, &location) {
                        continue;
                    }
                    if seen.insert(location.clone()) {
                        urls.push(MappedUrl {
                            url: location,
                            depth,
                            source,
                        });
                    }
                }
            }
            Err(error) => errors.push(format!("{sitemap}: {error}")),
        }
    }

    // No sitemap produced anything: read the links the seed page offers. A
    // failure here used to vanish, so a caller saw an empty map with no
    // reason; the reason now lands in the same list as the sitemap errors.
    let seed_body = if urls.is_empty() {
        match body(reader, seed).await {
            Ok(text) => Some(text),
            Err(error) => {
                errors.push(format!("{seed}: {error}"));
                None
            }
        }
    } else {
        None
    };
    if let Some(text) = seed_body {
        let document = Html::parse_document(&text);
        if let Ok(selector) = Selector::parse("a[href]") {
            for element in document.select(&selector).take(MAX_SEED_LINKS) {
                if urls.len() >= limit {
                    break;
                }
                let Some(href) = element.value().attr("href") else {
                    continue;
                };
                let Ok(resolved) = seed_url.join(href) else {
                    continue;
                };
                let Some(normalised) = normalize_url(resolved.as_str()) else {
                    continue;
                };
                if !same_origin(&seed_url, &normalised) {
                    continue;
                }
                if seen.insert(normalised.clone()) {
                    urls.push(MappedUrl {
                        url: normalised,
                        depth: 0,
                        source: UrlSource::SeedLinks,
                    });
                }
            }
        }
    }

    let truncated = urls.len() >= limit;
    Ok(MapReport {
        seed: seed.to_string(),
        origin,
        used_sitemap,
        truncated,
        urls,
        errors,
    })
}

/// One extracted record and where each field came from.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Record {
    /// The record's own identity, when the page names one.
    pub id: Option<String>,
    /// Field name to value.
    pub fields: std::collections::BTreeMap<String, String>,
    /// Where the record was found: `jsonld`, `definition-list`, `table`,
    /// or `document` for the reduced page text used as the last resort.
    pub source: &'static str,
    /// Path inside the source (JSON-LD key path, table row number, ...).
    pub path: String,
}

/// One record for a page that carries no structured data.
///
/// The text is the same main-content reduction the fetch path uses: the
/// largest article-like container when the page has one, the whole body text
/// otherwise — a prose page without semantic containers may still carry site
/// chrome in its text. A page with neither a title nor readable text yields
/// no record at all: an empty record would look like a match.
fn document_record(url: &str, document: &Html) -> Vec<Record> {
    let mut fields = std::collections::BTreeMap::new();
    if let Some(title) = document
        .select(&Selector::parse("title").expect("static selector"))
        .next()
    {
        let title: String = title.text().collect::<String>().trim().to_string();
        if !title.is_empty() {
            fields.insert(
                "title".to_string(),
                title.chars().take(MAX_FIELD_CHARS).collect(),
            );
        }
    }
    let text = crate::content::main_text(document);
    let text = text.trim();
    if text.is_empty() && fields.is_empty() {
        return Vec::new();
    }
    fields.insert("url".to_string(), url.to_string());
    if !text.is_empty() {
        fields.insert(
            "text".to_string(),
            text.chars().take(MAX_FIELD_CHARS).collect(),
        );
    }
    vec![Record {
        id: None,
        fields,
        source: "document",
        path: "body".to_string(),
    }]
}

/// The result of an `/extract` call.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExtractReport {
    pub url: String,
    /// Records found, in document order.
    pub records: Vec<Record>,
    /// Whether the list was cut at the limit.
    pub truncated: bool,
}

/// Flatten a JSON value into `path -> scalar` pairs.
fn flatten(
    value: &serde_json::Value,
    prefix: &str,
    out: &mut std::collections::BTreeMap<String, String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(value, &path, out);
            }
        }
        serde_json::Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let path = format!("{prefix}[{index}]");
                flatten(item, &path, out);
            }
        }
        serde_json::Value::String(text) => {
            let text = text.trim();
            if !text.is_empty() {
                out.insert(
                    prefix.to_string(),
                    text.chars().take(MAX_FIELD_CHARS).collect(),
                );
            }
        }
        serde_json::Value::Number(number) => {
            out.insert(prefix.to_string(), number.to_string());
        }
        serde_json::Value::Bool(flag) => {
            out.insert(prefix.to_string(), flag.to_string());
        }
        serde_json::Value::Null => {}
    }
}

/// Every `<script type="application/ld+json">` object on the page.
fn jsonld_records(document: &Html) -> Vec<Record> {
    let Ok(selector) = Selector::parse(r#"script[type="application/ld+json"]"#) else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for (index, element) in document.select(&selector).enumerate() {
        let raw = element.text().collect::<String>();
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        // A graph container holds several records; anything else is one record.
        if let Some(graph) = value.get("@graph").and_then(|g| g.as_array()) {
            for (position, item) in graph.iter().enumerate() {
                let mut fields = std::collections::BTreeMap::new();
                flatten(item, "", &mut fields);
                if fields.is_empty() {
                    continue;
                }
                records.push(Record {
                    id: item.get("@id").and_then(|v| v.as_str()).map(str::to_string),
                    path: format!("script[{index}].@graph[{position}]"),
                    fields,
                    source: "jsonld",
                });
            }
            continue;
        }
        let mut fields = std::collections::BTreeMap::new();
        flatten(&value, "", &mut fields);
        if fields.is_empty() {
            continue;
        }
        records.push(Record {
            id: value
                .get("@id")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            path: format!("script[{index}]"),
            fields,
            source: "jsonld",
        });
    }
    records
}

/// Definition lists as one record per `dt`/`dd` pair group.
fn definition_records(document: &Html) -> Vec<Record> {
    let (Ok(list_selector), Ok(term_selector)) = (Selector::parse("dl"), Selector::parse("dt, dd"))
    else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for (index, list) in document.select(&list_selector).enumerate() {
        let mut fields = std::collections::BTreeMap::new();
        let mut pending: Option<String> = None;
        for child in list.select(&term_selector) {
            let text = child.text().collect::<String>();
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                continue;
            }
            if child.value().name() == "dt" {
                pending = Some(text.chars().take(MAX_FIELD_CHARS).collect());
            } else if let Some(term) = pending.take() {
                let value: String = text.chars().take(MAX_FIELD_CHARS).collect();
                fields.insert(term, value);
            }
        }
        if !fields.is_empty() {
            records.push(Record {
                id: None,
                path: format!("dl[{index}]"),
                fields,
                source: "definition-list",
            });
        }
    }
    records
}

/// The first row of a table as field names, one record per following row.
fn table_records(document: &Html) -> Vec<Record> {
    let (Ok(table_selector), Ok(row_selector), Ok(cell_selector)) = (
        Selector::parse("table"),
        Selector::parse("tr"),
        Selector::parse("th, td"),
    ) else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for (index, table) in document.select(&table_selector).enumerate() {
        let mut rows = table.select(&row_selector);
        let Some(header) = rows.next() else {
            continue;
        };
        let names: Vec<String> = header
            .select(&cell_selector)
            .map(|cell| {
                cell.text()
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|name| !name.is_empty())
            .collect();
        if names.len() < 2 {
            continue;
        }
        for (row_index, row) in rows.enumerate() {
            if records.len() >= MAX_RECORDS {
                break;
            }
            let cells: Vec<String> = row
                .select(&cell_selector)
                .map(|cell| {
                    cell.text()
                        .collect::<String>()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect();
            if cells.len() < 2 {
                continue;
            }
            let mut fields = std::collections::BTreeMap::new();
            for (name, value) in names.iter().zip(cells.iter()) {
                if !value.is_empty() {
                    fields.insert(name.clone(), value.chars().take(MAX_FIELD_CHARS).collect());
                }
            }
            if !fields.is_empty() {
                records.push(Record {
                    id: None,
                    path: format!("table[{index}].row[{row_index}]"),
                    fields,
                    source: "table",
                });
            }
        }
    }
    records
}

/// Extract structured records from one page.
///
/// Sources are tried in order and the first that yields records wins, so a
/// page does not report the same data four times: JSON-LD, then definition
/// lists, then tables.
pub async fn extract(
    reader: &PageReader,
    url: &str,
    limit: usize,
) -> Result<ExtractReport, String> {
    let limit = limit.clamp(1, MAX_RECORDS);
    let text = body(reader, url).await?;
    let document = Html::parse_document(&text);
    let mut records = jsonld_records(&document);
    if records.is_empty() {
        records = definition_records(&document);
    }
    if records.is_empty() {
        records = table_records(&document);
    }
    // Prose pages carry no schema, and most pages an agent asks about are
    // prose. Returning nothing made the endpoint useless for the common case,
    // so the page's own text is the last resort — clearly labelled as such
    // rather than dressed up as extracted structure.
    if records.is_empty() {
        records = document_record(url, &document);
    }
    let truncated = records.len() > limit;
    records.truncate(limit);
    Ok(ExtractReport {
        url: url.to_string(),
        records,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prose_page_yields_the_document_text() {
        // The measured case: docs pages carry no schema, so /extract answered
        // with zero records and the endpoint was useless for the common page.
        let html = Html::parse_document(
            "<html><head><title>Pathlib</title></head><body>\
             <nav>Home | Docs</nav><main><p>Pathlib offers classes representing \
             filesystem paths with semantics appropriate for different operating \
             systems.</p></main></body></html>",
        );
        let records = document_record("https://example.test/", &html);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].source, "document");
        assert_eq!(
            records[0].fields.get("title").map(String::as_str),
            Some("Pathlib")
        );
        assert_eq!(
            records[0].fields.get("url").map(String::as_str),
            Some("https://example.test/")
        );
        let text = records[0].fields.get("text").expect("text");
        assert!(text.contains("filesystem paths"), "{text}");
        assert!(!text.contains("Home | Docs"), "navigation must be dropped");
    }

    #[test]
    fn a_page_without_text_or_title_yields_no_record() {
        let html = Html::parse_document("<html><body><nav></nav></body></html>");
        assert!(document_record("https://example.test/", &html).is_empty());
    }

    #[test]
    fn same_origin_is_an_origin_comparison_not_a_prefix_match() {
        let seed = reqwest::Url::parse("https://example.com/").unwrap();
        assert!(same_origin(&seed, "https://example.com/a/b?c=d"));
        assert!(same_origin(&seed, "https://example.com:443/x"));
        for candidate in [
            // A lookalike host whose name merely begins with the seed's host.
            "https://example.com.evil.example/x",
            "https://example.computer/x",
            "https://sub.example.com/x",
            // Same name, different port or scheme: still another origin.
            "https://example.com:8443/x",
            "http://example.com/x",
            // A trailing dot is a different hostname even though it resolves
            // to the same entity.
            "https://example.com./x",
            "not a url",
        ] {
            assert!(!same_origin(&seed, candidate), "{candidate}");
        }
    }

    #[tokio::test]
    async fn map_keeps_only_urls_on_the_seed_origin() {
        use axum::{Router, routing::get};
        let app = Router::new()
            .route(
                "/robots.txt",
                get(|| async {
                    "User-agent: *\nAllow: /\nSitemap: http://example.test/sitemap.xml\n"
                }),
            )
            .route(
                "/sitemap.xml",
                get(|| async {
                    (
                        [(axum::http::header::CONTENT_TYPE, "application/xml")],
                        concat!(
                            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
                            "<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">",
                            "<url><loc>http://example.test/kept</loc></url>",
                            "<url><loc>http://example.test.evil.example/x</loc></url>",
                            "<url><loc>http://example.testify/x</loc></url>",
                            "<url><loc>http://example.test:9/other-port</loc></url>",
                            "</urlset>"
                        ),
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (client, _) = crate::fetch::public_client(
            &crate::settings::Outgoing::default(),
            &[("example.test".into(), address)],
        )
        .unwrap();
        let reader = crate::fetch::PageReader::new(&crate::settings::Outgoing::default(), &[])
            .unwrap()
            .with_test_client(client);
        let report = map_site(&reader, "http://example.test/", 100)
            .await
            .unwrap();
        server.abort();
        assert!(
            report.errors.is_empty() && report.used_sitemap,
            "errors: {:?}, urls: {:?}",
            report.errors,
            report.urls
        );
        assert_eq!(
            report
                .urls
                .iter()
                .map(|u| u.url.as_str())
                .collect::<Vec<_>>(),
            ["http://example.test/kept"]
        );
    }

    #[tokio::test]
    async fn extract_reads_the_markup_not_the_flattened_text() {
        // Regression: /extract used to parse the reader's extracted prose, so
        // <script type="ld+json"> never survived to the record builders and
        // every page fell through to the document record.
        use axum::{Router, routing::get};
        let app = Router::new()
            .route(
                "/robots.txt",
                get(|| async { "User-agent: *\nAllow: /\n" }),
            )
            .route(
                "/page",
                get(|| async {
                    concat!(
                        "<html><head><title>Spec</title>",
                        r#"<script type="application/ld+json">{"@type":"TechArticle","name":"RustXNG","author":{"name":"Sahen"}}</script>"#,
                        "</head><body><main><p>Body text.</p></main></body></html>"
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (client, _) = crate::fetch::public_client(
            &crate::settings::Outgoing::default(),
            &[("example.test".into(), address)],
        )
        .unwrap();
        let reader = crate::fetch::PageReader::new(&crate::settings::Outgoing::default(), &[])
            .unwrap()
            .with_test_client(client);
        let report = extract(&reader, "http://example.test/page", 10)
            .await
            .unwrap();
        server.abort();
        assert_eq!(report.records.len(), 1, "{:?}", report.records);
        assert_eq!(report.records[0].source, "jsonld");
        assert_eq!(
            report.records[0].fields.get("name").map(String::as_str),
            Some("RustXNG")
        );
        assert_eq!(
            report.records[0]
                .fields
                .get("author.name")
                .map(String::as_str),
            Some("Sahen")
        );
    }

    #[test]
    fn jsonld_graph_yields_one_record_per_node() {
        let html = Html::parse_document(
            r##"<html><body><script type="application/ld+json">
            {"@graph":[
              {"@type":"Article","@id":"#a","name":"First","author":{"name":"Ada"}},
              {"@type":"Article","@id":"#b","name":"Second"}
            ]}</script></body></html>"##,
        );
        let records = jsonld_records(&html);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].id.as_deref(), Some("#a"));
        assert_eq!(
            records[0].fields.get("name").map(String::as_str),
            Some("First")
        );
        assert_eq!(
            records[0].fields.get("author.name").map(String::as_str),
            Some("Ada"),
            "nested objects are flattened into a path"
        );
    }

    #[test]
    fn a_bare_jsonld_object_is_one_record() {
        let html = Html::parse_document(
            r#"<script type="application/ld+json">{"name":"Solo","price":"3"}</script>"#,
        );
        let records = jsonld_records(&html);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].fields.get("price").map(String::as_str),
            Some("3")
        );
    }

    #[test]
    fn broken_jsonld_is_skipped_rather_than_failing_the_page() {
        let html = Html::parse_document(
            r#"<script type="application/ld+json">{not json}</script><script type="application/ld+json">{"name":"ok"}</script>"#,
        );
        let records = jsonld_records(&html);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].fields.get("name").map(String::as_str),
            Some("ok")
        );
    }

    #[test]
    fn a_definition_list_pairs_terms_with_definitions() {
        let html = Html::parse_document(
            "<dl><dt>Port</dt><dd>29483</dd><dt>Bind</dt><dd>127.0.0.1</dd></dl>",
        );
        let records = definition_records(&html);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].fields.get("Port").map(String::as_str),
            Some("29483")
        );
        assert_eq!(
            records[0].fields.get("Bind").map(String::as_str),
            Some("127.0.0.1")
        );
    }

    #[test]
    fn a_table_uses_its_header_as_field_names() {
        let html = Html::parse_document(
            "<table><tr><th>Name</th><th>Default</th></tr>             <tr><td>--port</td><td>29483</td></tr>             <tr><td>--bind</td><td>127.0.0.1</td></tr></table>",
        );
        let records = table_records(&html);
        assert_eq!(records.len(), 2);
        assert_eq!(
            records[0].fields.get("Name").map(String::as_str),
            Some("--port")
        );
        assert_eq!(
            records[1].fields.get("Default").map(String::as_str),
            Some("127.0.0.1")
        );
        assert_eq!(records[0].path, "table[0].row[0]");
    }

    #[test]
    fn a_single_column_table_is_not_mistaken_for_records() {
        let html =
            Html::parse_document("<table><tr><th>Only</th></tr><tr><td>one</td></tr></table>");
        assert!(table_records(&html).is_empty());
    }

    #[test]
    fn scalars_are_flattened_with_their_paths() {
        let value = serde_json::json!({"a": {"b": [1, "two", null]}, "c": true});
        let mut fields = std::collections::BTreeMap::new();
        flatten(&value, "", &mut fields);
        assert_eq!(fields.get("a.b[0]").map(String::as_str), Some("1"));
        assert_eq!(fields.get("a.b[1]").map(String::as_str), Some("two"));
        assert_eq!(fields.get("c").map(String::as_str), Some("true"));
        assert!(!fields.contains_key("a.b[2]"), "null contributes nothing");
    }
}
