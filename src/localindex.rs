//! Local full-text index (BM25) built from our own crawl.
//!
//! This is the piece that makes RustXNG independent of upstream engines: a
//! query answered from here costs milliseconds instead of a fan-out.
//!
//! Two tokenisations are indexed for every page, because the crawler cannot
//! assume a language: the default tokenizer for space-separated text, and
//! character bigrams for Japanese/Chinese, which have no word delimiters.

use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, BoostQuery, Occur, Query, QueryParser, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::tokenizer::{LowerCaser, NgramTokenizer, TextAnalyzer};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, Term, doc};

/// One page ready to be indexed.
pub struct Page<'a> {
    pub url: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub lang: &'a str,
    /// Short excerpt shown in result lists. Empty falls back to the start of
    /// the body.
    pub snippet: &'a str,
}

/// One search hit.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub score: f32,
}

/// Weight of a title match against a body match.
const TITLE_BOOST: f32 = 3.0;
/// Weight of a match in the words of the URL (host and path).
const URL_BOOST: f32 = 1.5;
/// Extra weight for pages that contain the whole query as a phrase.
const PHRASE_BOOST: f32 = 2.0;
/// Longest stored snippet, in characters.
const SNIPPET_CHARS: usize = 300;
/// Bumped whenever the schema changes; an older index must be rebuilt.
pub const SCHEMA_VERSION: u32 = 2;

/// A name for the CJK tokenizer, registered once per index.
const BIGRAM: &str = "bigram";

/// Fields of the schema, kept together so the searcher can address them.
struct Fields {
    url: Field,
    title: Field,
    body: Field,
    title_ja: Field,
    body_ja: Field,
    lang: Field,
    url_text: Field,
    snippet: Field,
}

/// An opened local index.
pub struct LocalIndex {
    index: Index,
    reader: IndexReader,
    fields: Fields,
    /// Present only for the process that writes the index.
    writer: Option<std::sync::Mutex<IndexWriter>>,
}

fn build_schema() -> Schema {
    let mut builder = Schema::builder();
    // The plain tokenizer for space-separated languages.
    let plain = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("default")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    let plain_url = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("default")
            .set_index_option(IndexRecordOption::WithFreqs),
    );
    // Bigrams for languages without spaces.
    let ngram = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(BIGRAM)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    builder.add_text_field("url", STRING | STORED);
    builder.add_text_field("title", plain.clone() | STORED);
    builder.add_text_field("body", plain);
    builder.add_text_field("title_ja", ngram.clone() | STORED);
    builder.add_text_field("body_ja", ngram);
    builder.add_text_field("lang", STRING | STORED);
    // The words of the host and path, so navigational queries ("tokio
    // tutorial") match pages whose URL says so.
    builder.add_text_field("url_text", plain_url);
    builder.add_text_field("snippet", STORED);
    builder.build()
}

/// The text for the bigram fields: only pages that contain Japanese or Chinese
/// get bigrams. Every letter pair of an English page would otherwise be
/// indexed for queries it can never answer, which doubles the index.
fn cjk_only(text: &str) -> String {
    if has_cjk(text) {
        text.to_string()
    } else {
        String::new()
    }
}

/// True when the text contains Japanese or Chinese characters.
pub fn has_cjk(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c as u32, 0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff))
}

/// The host and path of a URL as plain words ("docs.rs/tokio/latest" becomes
/// "docs rs tokio latest"); the default tokenizer splits on punctuation.
fn url_words(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(parsed) => format!(
            "{} {}",
            parsed.host_str().unwrap_or(""),
            parsed.path().replace(['/', '-', '_', '.'], " ")
        ),
        Err(_) => url.to_string(),
    }
}

impl LocalIndex {
    /// Open the index for reading. Safe to call from any number of processes,
    /// including while a crawl is writing.
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_inner(path, false)
    }

    /// Open the index for writing. Only one process may hold this at a time.
    pub fn open_for_write(path: &Path) -> Result<Self, String> {
        Self::open_inner(path, true)
    }

    fn open_inner(path: &Path, writable: bool) -> Result<Self, String> {
        // Only the writer creates the directory: a reader that finds nothing
        // must not litter the working tree with an empty index.
        if writable {
            std::fs::create_dir_all(path).map_err(|error| format!("create {path:?}: {error}"))?;
        } else if !path.exists() {
            return Err(format!("no index at {path:?}"));
        }
        let schema = build_schema();
        let index = match Index::open_in_dir(path) {
            Ok(index) => index,
            Err(_) => Index::create_in_dir(path, schema.clone())
                .map_err(|error| format!("create index: {error}"))?,
        };
        let tokenizer = TextAnalyzer::builder(
            NgramTokenizer::new(2, 2, false).map_err(|error| error.to_string())?,
        )
        .filter(LowerCaser)
        .build();
        index.tokenizers().register(BIGRAM, tokenizer);
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|error| format!("reader: {error}"))?;
        let writer = if writable {
            Some(
                index
                    .writer(256_000_000)
                    .map_err(|error| format!("writer: {error}"))?,
            )
        } else {
            None
        };
        let schema = index.schema();
        if schema.get_field("snippet").is_err() || schema.get_field("url_text").is_err() {
            return Err(format!(
                "the index at {path:?} was built by an older version (schema < {SCHEMA_VERSION}); rebuild it with `rustxng-index crawl`"
            ));
        }
        let fields = Fields {
            url: schema.get_field("url").map_err(|error| error.to_string())?,
            title: schema
                .get_field("title")
                .map_err(|error| error.to_string())?,
            body: schema
                .get_field("body")
                .map_err(|error| error.to_string())?,
            title_ja: schema
                .get_field("title_ja")
                .map_err(|error| error.to_string())?,
            body_ja: schema
                .get_field("body_ja")
                .map_err(|error| error.to_string())?,
            lang: schema
                .get_field("lang")
                .map_err(|error| error.to_string())?,
            url_text: schema
                .get_field("url_text")
                .map_err(|error| error.to_string())?,
            snippet: schema
                .get_field("snippet")
                .map_err(|error| error.to_string())?,
        };
        Ok(Self {
            index,
            reader,
            fields,
            writer: writer.map(std::sync::Mutex::new),
        })
    }

    /// Number of indexed documents.
    pub fn document_count(&self) -> u64 {
        self.reader.searcher().num_docs()
    }

    /// Whether this URL is already indexed (committed documents only).
    /// Lets a later crawl continue where the previous one stopped instead of
    /// indexing the same pages twice.
    pub fn has_url(&self, url: &str) -> bool {
        let searcher = self.reader.searcher();
        let query = TermQuery::new(
            Term::from_field_text(self.fields.url, url),
            IndexRecordOption::Basic,
        );
        searcher
            .search(&query, &TopDocs::with_limit(1).order_by_score())
            .map(|hits| !hits.is_empty())
            .unwrap_or(false)
    }

    /// Add one page. The caller decides when to commit.
    pub fn add(&self, page: &Page<'_>) -> Result<(), String> {
        let writer = self.writer.as_ref().ok_or("index opened read-only")?;
        let writer = writer.lock().map_err(|_| "writer poisoned")?;
        let snippet = if page.snippet.is_empty() {
            crate::crawl::snippet(page.body, SNIPPET_CHARS)
        } else {
            page.snippet.to_string()
        };
        writer
            .add_document(doc!(
                self.fields.url_text => url_words(page.url),
                self.fields.snippet => snippet,
                self.fields.url => page.url.to_string(),
                self.fields.title => page.title.to_string(),
                self.fields.body => page.body.to_string(),
                self.fields.title_ja => cjk_only(page.title),
                self.fields.body_ja => cjk_only(page.body),
                self.fields.lang => page.lang.to_string(),
            ))
            .map_err(|error| format!("add document: {error}"))?;
        Ok(())
    }

    /// Flush everything added so far and make it searchable.
    pub fn commit(&self) -> Result<(), String> {
        if let Some(writer) = self.writer.as_ref() {
            let mut writer = writer.lock().map_err(|_| "writer poisoned")?;
            writer
                .commit()
                .map_err(|error| format!("commit: {error}"))?;
        }
        self.reader
            .reload()
            .map_err(|error| format!("reload: {error}"))?;
        Ok(())
    }

    /// Rank the indexed pages for a query.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>, String> {
        self.search_with(query, limit, false)
    }

    /// Like [`search`](Self::search), but a page needs only one of the content
    /// words, and the ranking decides. For a large general index (an
    /// encyclopedia), where almost every question has a page that holds only
    /// some of its words.
    pub fn search_loose(&self, query: &str, limit: usize) -> Result<Vec<Hit>, String> {
        self.search_with(query, limit, true)
    }

    fn search_with(&self, query: &str, limit: usize, loose: bool) -> Result<Vec<Hit>, String> {
        let searcher = self.reader.searcher();
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        // Space-separated text goes through the standard parser.
        let mut parser = QueryParser::for_index(
            &self.index,
            vec![self.fields.title, self.fields.body, self.fields.url_text],
        );
        parser.set_field_boost(self.fields.title, TITLE_BOOST);
        parser.set_field_boost(self.fields.url_text, URL_BOOST);
        // Most of the content words must match. With plain OR matching a long
        // question matches any page that contains "the" or "of", and an index
        // of a few thousand pages would then crowd every answer.
        let content_words = content_words(trimmed);
        if !has_cjk(trimmed) && content_words.len() >= 2 {
            let per_word: Vec<(Occur, Box<dyn Query>)> = content_words
                .iter()
                .filter_map(|word| parser.parse_query(word).ok())
                .map(|query| (Occur::Should, query))
                .collect();
            let needed = if loose {
                1
            } else {
                required_matches(per_word.len())
            };
            if !per_word.is_empty() {
                clauses.push((
                    Occur::Should,
                    Box::new(BooleanQuery::with_minimum_required_clauses(
                        per_word, needed,
                    )),
                ));
            }
        } else if let Ok(parsed) = parser.parse_query(trimmed) {
            clauses.push((Occur::Should, parsed));
        }
        // A page that holds the whole query as a phrase is a better answer
        // than one that merely mentions every word somewhere.
        let words: Vec<&str> = trimmed
            .split_whitespace()
            .filter(|word| !word.contains(['"', ':', '(', ')']))
            .collect();
        if words.len() >= 2 && !has_cjk(trimmed) {
            if let Ok(phrase) = parser.parse_query(&format!("\"{}\"", words.join(" "))) {
                clauses.push((
                    Occur::Should,
                    Box::new(BoostQuery::new(phrase, PHRASE_BOOST)),
                ));
            }
        }
        // CJK text has no spaces: match character bigrams instead.
        if has_cjk(trimmed) {
            let lowered: Vec<char> = trimmed.to_lowercase().chars().collect();
            for pair in lowered.windows(2) {
                if !has_cjk(&pair.iter().collect::<String>()) {
                    continue;
                }
                let gram: String = pair.iter().collect();
                for field in [self.fields.title_ja, self.fields.body_ja] {
                    clauses.push((
                        Occur::Should,
                        Box::new(TermQuery::new(
                            Term::from_field_text(field, &gram),
                            IndexRecordOption::WithFreqs,
                        )),
                    ));
                }
            }
        }
        if clauses.is_empty() {
            return Ok(Vec::new());
        }
        let query = BooleanQuery::new(clauses);
        let top = searcher
            .search(&query, &TopDocs::with_limit(limit).order_by_score())
            .map_err(|error| format!("search: {error}"))?;
        let mut hits = Vec::new();
        for (score, address) in top {
            let document: tantivy::TantivyDocument = searcher
                .doc(address)
                .map_err(|error| format!("doc: {error}"))?;
            let text = |field: Field| {
                document
                    .get_first(field)
                    .and_then(|value| value.as_str().map(str::to_string))
                    .unwrap_or_default()
            };
            hits.push(Hit {
                url: text(self.fields.url),
                title: text(self.fields.title),
                snippet: text(self.fields.snippet),
                score,
            });
        }
        Ok(hits)
    }
}

/// Words too common to say anything about the topic of a query.
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "did", "do", "does", "for", "from", "has",
    "have", "how", "in", "is", "it", "its", "many", "most", "much", "of", "on", "or", "that",
    "the", "their", "there", "this", "to", "was", "were", "what", "when", "where", "which", "who",
    "whom", "whose", "why", "will", "with",
];

/// The distinct plain words of a query that carry meaning.
fn content_words(query: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for raw in query.split_whitespace() {
        if raw.contains(['"', ':', '(', ')']) {
            continue;
        }
        let word: String = raw
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        if word.is_empty() || STOP_WORDS.contains(&word.as_str()) || words.contains(&word) {
            continue;
        }
        words.push(word);
    }
    words
}

/// How many of `count` content words a page has to contain: all of one or two,
/// then about three out of five.
fn required_matches(count: usize) -> usize {
    match count {
        0 | 1 => count,
        2 => 2,
        _ => (count * 3).div_ceil(5),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_index() -> (TempDir, LocalIndex) {
        let dir = TempDir::new().unwrap();
        let index = LocalIndex::open_for_write(dir.path()).unwrap();
        (dir, index)
    }

    #[test]
    fn a_reader_does_not_create_the_directory() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("index-data");
        assert!(
            LocalIndex::open(&missing).is_err(),
            "a missing index is an error"
        );
        assert!(!missing.exists(), "the reader must not create it");
        // The writer creates it on demand.
        let index = LocalIndex::open_for_write(&missing).unwrap();
        index.commit().unwrap();
        assert!(missing.exists());
    }

    #[test]
    fn indexes_and_finds_english_pages() {
        let (_dir, index) = temp_index();
        index
            .add(&Page {
                url: "https://tokio.rs/tokio/tutorial",
                title: "Tutorial",
                body: "async runtime for rust with tasks and futures",
                lang: "en",
                snippet: "",
            })
            .unwrap();
        index
            .add(&Page {
                url: "https://example.com/other",
                title: "Other",
                body: "nothing to do with the query",
                lang: "en",
                snippet: "",
            })
            .unwrap();
        index.commit().unwrap();
        assert_eq!(index.document_count(), 2);
        let hits = index.search("async runtime rust", 5).unwrap();
        assert_eq!(hits[0].url, "https://tokio.rs/tokio/tutorial");
    }

    #[test]
    fn a_long_question_does_not_match_pages_that_share_only_common_words() {
        let (_dir, index) = temp_index();
        for (url, title, body) in [
            (
                "https://docs.example/rust-book",
                "Ownership in Rust",
                "the rules of ownership and borrowing decide how memory is moved",
            ),
            (
                "https://docs.example/unrelated",
                "Unrelated page",
                "this is the page of the most common words that were in the year",
            ),
        ] {
            index
                .add(&Page {
                    url,
                    title,
                    body,
                    lang: "en",
                    snippet: "",
                })
                .unwrap();
        }
        index.commit().unwrap();
        let off_topic = index
            .search(
                "Who holds the all-time record for the most wins in the album of the year category?",
                10,
            )
            .unwrap();
        assert!(
            off_topic.is_empty(),
            "common words alone must not match: {off_topic:?}"
        );
        let on_topic = index
            .search("what are the rules of ownership and borrowing in rust", 10)
            .unwrap();
        assert_eq!(on_topic[0].url, "https://docs.example/rust-book");
        assert_eq!(required_matches(5), 3);
        assert_eq!(
            content_words("The Rust, the rust!"),
            vec!["rust".to_string()]
        );
    }

    #[test]
    fn loose_search_finds_a_page_that_holds_only_some_of_the_words() {
        let (_dir, index) = temp_index();
        index
            .add(&Page {
                url: "https://en.wikipedia.org/wiki/Grammy_Award_for_Album_of_the_Year",
                title: "Grammy Award for Album of the Year",
                body: "The award goes to the artists whose album won; Taylor Swift holds the record.",
                lang: "en",
                snippet: "",
            })
            .unwrap();
        index.commit().unwrap();
        let question = "Which artist has the record for most Grammy wins in album of the year, and which engineer";
        assert!(index.search(question, 5).unwrap().is_empty());
        let loose = index.search_loose(question, 5).unwrap();
        assert_eq!(loose.len(), 1);
        assert!(loose[0].url.ends_with("Album_of_the_Year"));
    }

    #[test]
    fn indexes_and_finds_japanese_pages() {
        let (_dir, index) = temp_index();
        index
            .add(&Page {
                url: "https://www.gsi.go.jp/",
                title: "国土地理院",
                body: "地図と測量の情報を提供しています",
                lang: "ja",
                snippet: "",
            })
            .unwrap();
        index.commit().unwrap();
        let hits = index.search("国土地理院 地図", 5).unwrap();
        assert_eq!(hits[0].url, "https://www.gsi.go.jp/");
    }

    fn page<'a>(url: &'a str, title: &'a str, body: &'a str) -> Page<'a> {
        Page {
            url,
            title,
            body,
            lang: "en",
            snippet: "",
        }
    }

    #[test]
    fn a_title_match_beats_a_body_mention() {
        let (_dir, index) = temp_index();
        index
            .add(&page(
                "https://a.example/body",
                "Unrelated notes",
                "we sometimes talk about tantivy indexing in passing",
            ))
            .unwrap();
        index
            .add(&page(
                "https://b.example/title",
                "Tantivy indexing guide",
                "a guide",
            ))
            .unwrap();
        index.commit().unwrap();
        let hits = index.search("tantivy indexing", 5).unwrap();
        assert_eq!(hits[0].url, "https://b.example/title");
    }

    #[test]
    fn the_words_of_the_url_count() {
        let (_dir, index) = temp_index();
        index
            .add(&page("https://example.com/misc", "Page", "generic text"))
            .unwrap();
        index
            .add(&page(
                "https://docs.rs/serde-json/latest",
                "Page",
                "generic text",
            ))
            .unwrap();
        index.commit().unwrap();
        let hits = index.search("serde json", 5).unwrap();
        assert_eq!(hits[0].url, "https://docs.rs/serde-json/latest");
    }

    #[test]
    fn the_whole_phrase_outranks_scattered_words() {
        let (_dir, index) = temp_index();
        index
            .add(&page(
                "https://a.example/scattered",
                "Notes",
                "dict is a type. the get of other things. method names vary",
            ))
            .unwrap();
        index
            .add(&page(
                "https://b.example/phrase",
                "Notes",
                "use the dict get method to read a key safely",
            ))
            .unwrap();
        index.commit().unwrap();
        let hits = index.search("dict get method", 5).unwrap();
        assert_eq!(hits[0].url, "https://b.example/phrase");
    }

    #[test]
    fn hits_carry_a_snippet_from_the_body() {
        let (_dir, index) = temp_index();
        index
            .add(&page(
                "https://example.com/p",
                "Title",
                "The quick brown fox jumps over the lazy dog.",
            ))
            .unwrap();
        index.commit().unwrap();
        let hits = index.search("quick fox", 5).unwrap();
        assert!(hits[0].snippet.starts_with("The quick brown fox"));
    }

    #[test]
    fn an_index_from_an_older_schema_is_reported_clearly() {
        let dir = TempDir::new().unwrap();
        let mut builder = Schema::builder();
        builder.add_text_field("url", STRING | STORED);
        Index::create_in_dir(dir.path(), builder.build()).unwrap();
        let error = LocalIndex::open(dir.path()).err().expect("must fail");
        assert!(error.contains("older version"), "{error}");
    }
}
