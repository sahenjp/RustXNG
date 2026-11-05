//! Mwmbl engine, ported from `searx/engines/mwmbl.py`.
//!
//! Mwmbl is a small, non-profit search engine; upstream does not support
//! regions, languages, safe search or time ranges, and neither do we.
//!
//! The default endpoint is the documented v2 API
//! (<https://developer.mwmbl.org/terms/>): anonymous use is allowed at 1
//! request per second and 1,000 requests per month; an `api_key` raises that
//! (sent as `X-API-Key`). A `base_url` ending in `/v1` selects the older
//! response shape, which self-hosted instances and tests still use.

use std::sync::Arc;

use crate::engine::util::*;
use crate::engine::{
    Engine, EngineDefaults, EngineError, EngineItem, EngineMeta, EngineRequest, EngineResponse,
};
use crate::query::Query;
use crate::settings::EngineConfig;

const DEFAULTS: EngineDefaults = EngineDefaults {
    categories: &["general"],
    shortcut: "mwm",
    paging: false,
    time_range_support: false,
    safesearch: false,
    language_support: false,
    weight: 1.0,
    timeout: 4.0,
};

/// Longest query sent, in words (see `search_few_words`).
const MAX_WORDS: usize = 4;

const API_URL: &str = "https://api.mwmbl.org/api/v2";

/// Mwmbl.
pub struct Mwmbl {
    meta: EngineMeta,
}

impl Mwmbl {
    pub fn from_config(config: &EngineConfig) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            meta: EngineMeta::from_config(config, DEFAULTS),
        }))
    }
}

impl Engine for Mwmbl {
    fn meta(&self) -> &EngineMeta {
        &self.meta
    }

    fn build_request(&self, query: &Query, _pageno: u32) -> Result<EngineRequest, EngineError> {
        let base = self
            .meta
            .base_url
            .clone()
            .unwrap_or_else(|| API_URL.to_string());
        let base = base.trim_end_matches('/');
        // Mwmbl needs every word to match and its index is small: a sentence
        // returns nothing, a handful of topic words usually something.
        let text = encode(&search_few_words(&query.query_text(), MAX_WORDS));
        let mut request = if base.ends_with("/v1") {
            EngineRequest::get(format!("{base}/search/?s={text}"))
        } else {
            EngineRequest::get(format!("{base}/search/?q={text}"))
        };
        if let Some(key) = self.meta.api_key.as_deref().map(str::trim) {
            if !key.is_empty() {
                request.set_header("X-API-Key", key);
            }
        }
        Ok(request)
    }

    fn parse(
        &self,
        response: &EngineResponse,
        _query: &Query,
    ) -> Result<Vec<EngineItem>, EngineError> {
        let json = response.json()?;
        // v2 wraps the list in {"results": [...]} with plain-text fields;
        // v1 is a bare array of highlighted fragments.
        let items = json
            .as_array()
            .or_else(|| json.get("results").and_then(|value| value.as_array()))
            .ok_or_else(|| EngineError::Parse("expected a JSON array".into()))?;
        let public_api = url::Url::parse(&response.final_url)
            .ok()
            .is_some_and(|url| url.host_str() == Some("api.mwmbl.org"));
        let mut results = Vec::new();
        for item in items {
            let url = json_str(item, "url");
            if url.is_empty() {
                continue;
            }
            let content = match item.get("extract") {
                Some(_) => join_fragments(item, "extract"),
                None => join_fragments(item, "content"),
            };
            let mut result =
                ResultBuilder::new(url, join_fragments(item, "title")).content(content);
            if public_api {
                result = result
                    .metadata("source_license", "CC BY-NC-SA 4.0")
                    .metadata(
                        "source_license_url",
                        "https://creativecommons.org/licenses/by-nc-sa/4.0/",
                    )
                    .metadata("source_terms_url", "https://developer.mwmbl.org/terms/");
            }
            results.push(result.build());
        }
        Ok(results)
    }
}

/// Join the `[{"value": "..."}]` fragments Mwmbl returns for text fields.
pub fn join_fragments(item: &serde_json::Value, key: &str) -> String {
    match item.get(key) {
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .map(|part| json_str(part, "value"))
            .collect::<Vec<_>>()
            .join(""),
        Some(serde_json::Value::String(text)) => text.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineRegistry;

    fn query(raw: &str) -> Query {
        Query::parse(raw, &EngineRegistry::default(), &[])
    }

    fn engine(base_url: Option<&str>, api_key: Option<&str>) -> Arc<Mwmbl> {
        let mut config = EngineConfig::new("mwmbl", "mwmbl", "mwm", &["general"]);
        config.base_url = base_url.map(str::to_string);
        config.api_key = api_key.map(str::to_string);
        Mwmbl::from_config(&config).unwrap()
    }

    #[test]
    fn the_default_endpoint_is_v2_and_needs_no_key() {
        let query = query("rust language");
        let request = engine(None, None).build_request(&query, 1).unwrap();
        assert_eq!(
            request.url,
            "https://api.mwmbl.org/api/v2/search/?q=rust%20language"
        );
        assert!(request.headers.get("x-api-key").is_none());
    }

    #[test]
    fn an_api_key_is_sent_as_a_header() {
        let query = query("rust");
        let request = engine(None, Some(" abc "))
            .build_request(&query, 1)
            .unwrap();
        assert_eq!(request.headers.get("x-api-key").unwrap(), "abc");
    }

    #[test]
    fn a_v1_base_url_keeps_the_old_request_shape() {
        let query = query("rust");
        let request = engine(Some("http://h/api/v1"), None)
            .build_request(&query, 1)
            .unwrap();
        assert_eq!(request.url, "http://h/api/v1/search/?s=rust");
    }

    #[test]
    fn v2_and_v1_responses_both_parse() {
        let query = query("rust");
        let v2 = EngineResponse::new(
            200,
            API_URL,
            r#"{"results":[{"url":"https://a.example/","title":"A","content":"about a"}]}"#
                .as_bytes()
                .to_vec(),
        );
        let items = engine(None, None).parse(&v2, &query).unwrap();
        assert_eq!(items.len(), 1);
        let EngineItem::Result(result) = &items[0] else {
            panic!("expected result");
        };
        assert_eq!(result.metadata["source_license"], "CC BY-NC-SA 4.0");
        let v1 = EngineResponse::new(
            200,
            API_URL,
            r#"[{"url":"https://b.example/","title":[{"value":"B"}],"extract":[{"value":"about b"}]}]"#
                .as_bytes()
                .to_vec(),
        );
        assert_eq!(engine(None, None).parse(&v1, &query).unwrap().len(), 1);
    }

    #[test]
    fn a_custom_instance_does_not_inherit_the_public_apis_license() {
        let response = EngineResponse::new(
            200,
            "https://private.example/api/v2/search/",
            br#"{"results":[{"url":"https://a.example/","title":"A","content":"about a"}]}"#
                .to_vec(),
        );
        let items = engine(Some("https://private.example/api/v2"), None)
            .parse(&response, &query("rust"))
            .unwrap();
        let EngineItem::Result(result) = &items[0] else {
            panic!("expected result");
        };
        assert!(!result.metadata.contains_key("source_license"));
    }
}
