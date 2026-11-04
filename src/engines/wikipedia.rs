//! Wikipedia engine, ported from `searx/engines/wikipedia.py`.
//!
//! The MediaWiki search API is used (upstream additionally fetches the REST
//! summary for the infobox; that part is documented as not ported).

use std::sync::Arc;

use crate::engine::util::*;
use crate::engine::{
    Engine, EngineDefaults, EngineError, EngineItem, EngineMeta, EngineRequest, EngineResponse,
};
use crate::query::Query;
use crate::settings::EngineConfig;

const DEFAULTS: EngineDefaults = EngineDefaults {
    categories: &["general"],
    shortcut: "wp",
    paging: true,
    time_range_support: false,
    safesearch: false,
    language_support: true,
    weight: 1.0,
    timeout: 4.0,
};

/// Wikipedia.
pub struct Wikipedia {
    meta: EngineMeta,
    /// How a long question is turned into a search.
    mode: Mode,
}

/// The shape of the search an engine sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Every topic word must match (`wikipedia`).
    Keywords,
    /// The named entities, as quoted phrases (`wikipedia_entities`).
    Entities,
    /// Any of the topic words, best match first (`wikipedia_any`).
    AnyWords,
}

impl Wikipedia {
    pub fn from_config(config: &EngineConfig) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            meta: EngineMeta::from_config(config, DEFAULTS),
            mode: Mode::Keywords,
        }))
    }

    /// The companion engine that quotes the entities of a long question.
    /// Without any it asks the same thing as `wikipedia`, which the response
    /// cache answers without a second request.
    pub fn entities_from_config(config: &EngineConfig) -> Result<Arc<Self>, String> {
        Self::companion(config, "wpe", Mode::Entities)
    }

    /// The companion engine that matches any of a long question's words.
    pub fn any_words_from_config(config: &EngineConfig) -> Result<Arc<Self>, String> {
        Self::companion(config, "wpa", Mode::AnyWords)
    }

    fn companion(
        config: &EngineConfig,
        shortcut: &'static str,
        mode: Mode,
    ) -> Result<Arc<Self>, String> {
        let mut defaults = DEFAULTS;
        defaults.shortcut = shortcut;
        defaults.paging = false;
        Ok(Arc::new(Self {
            meta: EngineMeta::from_config(config, defaults),
            mode,
        }))
    }

    /// The wiki host for a language, e.g. `ja.wikipedia.org`.
    fn host(&self, query: &Query) -> String {
        if let Some(base) = &self.meta.base_url {
            return base.trim_end_matches('/').to_string();
        }
        let language = locale_language(&query.locale);
        // No language chosen: follow the query's script (Japanese text goes
        // to the Japanese wiki), as SearXNG's "auto" does.
        let language = if language == "all" {
            query.engine_language()
        } else {
            language
        };
        format!("https://{language}.wikipedia.org")
    }
}

impl Wikipedia {
    fn search_text(&self, query: &Query) -> String {
        let text = without_site_name(&query.query_text());
        let special = match self.mode {
            Mode::Keywords => None,
            Mode::Entities => search_entities(&text),
            Mode::AnyWords => search_any_words(&text),
        };
        special.unwrap_or_else(|| search_keywords(&text))
    }
}

/// A title as MediaWiki writes it in article URLs (`wfUrlencode`): spaces
/// become underscores and `;@$!*(),/~:` stay literal.
pub fn wiki_title_path(title: &str) -> String {
    const KEEP: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~')
        .remove(b';')
        .remove(b'@')
        .remove(b'$')
        .remove(b'!')
        .remove(b'*')
        .remove(b'(')
        .remove(b')')
        .remove(b',')
        .remove(b'/')
        .remove(b':');
    percent_encoding::utf8_percent_encode(&title.replace(' ', "_"), KEEP).to_string()
}

/// Drop the word "wikipedia" from a query sent to Wikipedia: it names the
/// site, not the topic, and Wikipedia's search requires every word.
/// ("富士山 wikipedia" finds the volcano; with the word it finds odd pages.)
pub fn without_site_name(text: &str) -> String {
    let kept: Vec<&str> = text
        .split_whitespace()
        .filter(|word| {
            !matches!(
                word.to_lowercase().as_str(),
                "wikipedia" | "ウィキペディア" | "维基百科" | "維基百科"
            )
        })
        .collect();
    if kept.is_empty() || kept.len() == text.split_whitespace().count() {
        return text.to_string();
    }
    kept.join(" ")
}

impl Engine for Wikipedia {
    fn meta(&self) -> &EngineMeta {
        &self.meta
    }

    /// The companions only run when they would ask something different from
    /// the plain search; otherwise they repeat its results and triple-count
    /// Wikipedia in the merge.
    fn is_applicable(&self, query: &Query) -> bool {
        let text = without_site_name(&query.query_text());
        match self.mode {
            Mode::Keywords => true,
            Mode::Entities => search_entities(&text).is_some(),
            Mode::AnyWords => search_any_words(&text).is_some(),
        }
    }

    fn build_request(&self, query: &Query, pageno: u32) -> Result<EngineRequest, EngineError> {
        let pairs: Vec<(&str, String)> = vec![
            ("action", "query".to_string()),
            ("list", "search".to_string()),
            ("srsearch", self.search_text(query)),
            ("srlimit", "10".to_string()),
            ("sroffset", ((pageno - 1) * 10).to_string()),
            ("format", "json".to_string()),
        ];
        Ok(EngineRequest::get(format!(
            "{}/w/api.php?{}",
            self.host(query),
            query_string(&pairs)
        ))
        .header("Accept", "application/json"))
    }

    fn parse(
        &self,
        response: &EngineResponse,
        query: &Query,
    ) -> Result<Vec<EngineItem>, EngineError> {
        let json = response.json()?;
        let items = match json_path(&json, "query.search").and_then(|value| value.as_array()) {
            Some(items) => items,
            None => return Ok(Vec::new()),
        };
        let host = self.host(query);
        let mut results = Vec::new();
        for item in items {
            let title = json_str(item, "title");
            if title.is_empty() {
                continue;
            }
            // The readable article URL: it is what other engines return, so
            // duplicates merge, and what a reader expects to see.
            // The canonical form other engines link to (`Rust_(programming_language)`),
            // so the same article from two sources merges into one result.
            let url = format!("{host}/wiki/{}", wiki_title_path(&title));
            results.push(
                ResultBuilder::new(url, title)
                    .content(strip_tags(&json_str(item, "snippet")))
                    .build(),
            );
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineRegistry;

    fn request_url(text: &str, locale: &str) -> String {
        let config = EngineConfig::new("wikipedia", "wikipedia", "wp", &["general"]);
        let engine = Wikipedia::from_config(&config).unwrap();
        let mut query = Query::parse(text, &EngineRegistry::default(), &[]);
        query.locale = locale.to_string();
        engine.build_request(&query, 1).unwrap().url
    }

    #[test]
    fn article_urls_use_the_canonical_encoding() {
        assert_eq!(
            wiki_title_path("Rust (programming language)"),
            "Rust_(programming_language)"
        );
        assert_eq!(wiki_title_path("Hello, world"), "Hello,_world");
        assert_eq!(wiki_title_path("AT&T?"), "AT%26T%3F");
        assert_eq!(wiki_title_path("気象庁"), "%E6%B0%97%E8%B1%A1%E5%BA%81");
    }

    #[test]
    fn companions_only_run_when_they_change_the_query() {
        let config = EngineConfig::new("wikipedia any words", "wikipedia_any", "wpa", &["general"]);
        let any = Wikipedia::any_words_from_config(&config).unwrap();
        let short = Query::parse(
            "fastapi documentation tutorial",
            &EngineRegistry::default(),
            &[],
        );
        assert!(!any.is_applicable(&short));
        let long = Query::parse(
            "Who holds the all-time record at the Grammys for the most wins?",
            &EngineRegistry::default(),
            &[],
        );
        assert!(any.is_applicable(&long));
    }

    #[test]
    fn the_site_name_is_not_searched_for() {
        assert_eq!(without_site_name("富士山 wikipedia"), "富士山");
        assert_eq!(without_site_name("Alan Turing Wikipedia"), "Alan Turing");
        assert_eq!(without_site_name("wikipedia"), "wikipedia");
        assert_eq!(
            without_site_name("rust \"wikipedia\""),
            "rust \"wikipedia\""
        );
    }

    #[test]
    fn without_a_language_the_wiki_follows_the_query_script() {
        assert!(request_url("富士山", "").starts_with("https://ja.wikipedia.org/"));
        assert!(request_url("mount fuji", "").starts_with("https://en.wikipedia.org/"));
        // A chosen language still wins.
        assert!(request_url("富士山", "en-US").starts_with("https://en.wikipedia.org/"));
    }
}
