// SPDX-License-Identifier: AGPL-3.0-or-later
//! Qiita engine (no upstream searx equivalent).
// engine-id: qiita
//!
//! Qiita exposes a public search API at `/api/v2/items` that returns JSON
//! without an API key. Unauthenticated requests are limited to 60 per hour, so
//! the engine is disabled by default. Enable it explicitly for the `it` category.
//! Setting `api_key` (a personal access token) raises the limit to 1,000 per
//! hour.
//!
//! The API ignores most relevance settings, so `query` is forwarded verbatim.
//! Callers that want title matches pass Qiita's own syntax (`term in:title`).

use std::sync::Arc;

use crate::engine::util::*;
use crate::engine::{
    Engine, EngineDefaults, EngineError, EngineItem, EngineMeta, EngineRequest, EngineResponse,
};
use crate::query::Query;
use crate::settings::EngineConfig;

const DEFAULTS: EngineDefaults = EngineDefaults {
    categories: &["it"],
    shortcut: "qi",
    paging: true,
    time_range_support: false,
    safesearch: false,
    language_support: false,
    weight: 1.0,
    timeout: 4.0,
};

const SEARCH_URL: &str = "https://qiita.com/api/v2/items";
const PAGE_SIZE: u32 = 20;
/// The API returns the whole article body; keep the snippet short.
const SNIPPET_CHARS: usize = 240;

/// Qiita, the Japanese developer knowledge base.
pub struct Qiita {
    meta: EngineMeta,
}

impl Qiita {
    pub fn from_config(config: &EngineConfig) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            meta: EngineMeta::from_config(config, DEFAULTS),
        }))
    }

    fn api_key(&self) -> Option<&str> {
        self.meta
            .api_key
            .as_deref()
            .or_else(|| self.meta.opt_str("api_key"))
            .map(str::trim)
            .filter(|key| !key.is_empty())
    }
}

impl Engine for Qiita {
    fn meta(&self) -> &EngineMeta {
        &self.meta
    }

    fn build_request(&self, query: &Query, pageno: u32) -> Result<EngineRequest, EngineError> {
        let page = pageno.max(1);
        if page > 100 {
            return Err(EngineError::Message(
                "qiita supports pages 1 through 100".into(),
            ));
        }
        let pairs: Vec<(&str, String)> = vec![
            ("query", query.query_text()),
            ("per_page", PAGE_SIZE.to_string()),
            ("page", page.to_string()),
        ];
        let mut request = EngineRequest::get(format!("{SEARCH_URL}?{}", query_string(&pairs)))
            .header("Accept", "application/json");
        if let Some(key) = self.api_key() {
            let bearer = format!("Bearer {key}");
            request = request.header("Authorization", &bearer);
        }
        Ok(request)
    }

    fn parse(
        &self,
        response: &EngineResponse,
        _query: &Query,
    ) -> Result<Vec<EngineItem>, EngineError> {
        let json = response.json()?;
        let Some(items) = json.as_array() else {
            return Ok(Vec::new());
        };
        let mut results = Vec::new();
        for item in items {
            let title = clean(&json_str(item, "title"));
            let url = json_str(item, "url");
            if title.is_empty() || url.is_empty() {
                continue;
            }
            let tags = json_path(item, "tags")
                .and_then(serde_json::Value::as_array)
                .map(|tags| {
                    tags.iter()
                        .filter_map(|tag| tag.get("name").and_then(serde_json::Value::as_str))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            results.push(
                ResultBuilder::new(url, title)
                    .content(snippet(&json_str(item, "body")))
                    .author(json_str(item, "user.id"))
                    .published_date(date_part(&json_str(item, "created_at")))
                    .metadata("tags", tags)
                    .metadata("likes", json_str(item, "likes_count"))
                    .metadata("comments", json_str(item, "comments_count"))
                    .metadata("updated_at", json_str(item, "updated_at"))
                    .build(),
            );
        }
        Ok(results)
    }
}

/// `2026-11-02T01:51:03+09:00` -> `2026-11-02`.
fn date_part(value: &str) -> String {
    value
        .split_once('T')
        .map(|(date, _)| date.to_string())
        .unwrap_or_default()
}

/// Collapse the markdown body and cut it to [`SNIPPET_CHARS`] characters.
fn snippet(body: &str) -> String {
    let cleaned = clean(body);
    match cleaned.char_indices().nth(SNIPPET_CHARS) {
        Some((cut, _)) => format!("{}…", cleaned[..cut].trim_end()),
        None => cleaned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r########"[
      {
        "id": "39f0654dcd771a2af1fb",
        "title": "[Typst] 大学のレポート作成を効率化するため自作コマンドを作った話",
        "url": "https://qiita.com/example/items/39f0654dcd771a2af1fb",
        "body": "## はじめに\n\nTypst は組版システムです。\n\n```typ\n#set text(font: \"Noto Sans JP\")\n```",
        "created_at": "2026-10-30T12:00:00+09:00",
        "updated_at": "2026-10-31T09:30:00+09:00",
        "likes_count": 12,
        "comments_count": 3,
        "tags": [{"name": "Typst"}, {"name": "Linux"}],
        "user": {"id": "example"}
      },
      {
        "id": "abc",
        "title": "",
        "url": "https://qiita.com/example/items/abc",
        "body": "",
        "user": {"id": "example"}
      }
    ]"########;

    fn engine() -> Arc<Qiita> {
        Qiita::from_config(&EngineConfig::new("qiita", "qiita", "qi", &["it"])).unwrap()
    }

    fn query() -> Query {
        Query::parse("typst", &crate::engine::EngineRegistry::default(), &[])
    }

    #[test]
    fn parses_articles() {
        let response = EngineResponse::new(200, SEARCH_URL, FIXTURE.as_bytes().to_vec());
        let items = engine().parse(&response, &query()).unwrap();
        assert_eq!(items.len(), 1, "empty titles are skipped");
        match &items[0] {
            EngineItem::Result(result) => {
                assert_eq!(
                    result.url,
                    "https://qiita.com/example/items/39f0654dcd771a2af1fb"
                );
                assert_eq!(result.author.as_deref(), Some("example"));
                assert_eq!(result.published_date.as_deref(), Some("2026-10-30"));
                assert_eq!(
                    result.metadata.get("tags").map(String::as_str),
                    Some("Typst, Linux")
                );
                assert_eq!(result.metadata.get("likes").map(String::as_str), Some("12"));
                assert!(result.content.starts_with("## はじめに"));
                assert!(result.content.contains("Typst は組版システムです。"));
            }
            _ => panic!("expected a result"),
        }
    }

    #[test]
    fn snippet_is_cut_on_a_char_boundary() {
        let long = "あ".repeat(SNIPPET_CHARS + 50);
        let cut = snippet(&long);
        assert_eq!(
            cut.chars().count(),
            SNIPPET_CHARS + 1,
            "one extra for the ellipsis"
        );
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn pagination_respects_the_api_limit() {
        for page in [0, 1, 100] {
            assert!(engine().build_request(&query(), page).is_ok());
        }
        for page in [101, u32::MAX] {
            assert!(engine().build_request(&query(), page).is_err());
        }
        assert!(
            crate::settings::Settings::defaults()
                .engine("qiita")
                .unwrap()
                .disabled
                .unwrap_or(false)
        );
    }

    #[test]
    fn builds_a_paged_request() {
        let request = engine().build_request(&query(), 2).unwrap();
        assert!(request.url.starts_with(SEARCH_URL));
        assert!(request.url.contains("per_page=20"));
        assert!(request.url.contains("page=2"));
        assert!(request.url.contains("query=typst"));
    }
}
