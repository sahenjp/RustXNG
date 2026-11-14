//! Search suggestions, ported from `searx/autocomplete.py`.

use std::collections::HashSet;

use crate::engine::util::{json_str, locale_language};
use crate::engine::{EngineRequest, EngineResponse};
use crate::network::Network;
use crate::settings::Settings;

/// Autocomplete backends, in the order they are tried.
pub static BACKENDS: &[&str] = &["duckduckgo", "wikipedia", "mwmbl"];

/// Fetch suggestions for `query` from `backend`.
pub async fn suggest(
    network: &Network,
    settings: &Settings,
    backend: &str,
    query: &str,
    locale: &str,
) -> Vec<String> {
    let (url, is_json) = match backend {
        "wikipedia" => {
            let language = locale_language(locale);
            let language = if language == "all" {
                "en".to_string()
            } else {
                language
            };
            (
                format!(
                    "https://{language}.wikipedia.org/w/api.php?action=opensearch&limit=10&format=json&search={}",
                    crate::engine::util::encode(query)
                ),
                true,
            )
        }
        "mwmbl" => (
            format!(
                "https://api.mwmbl.org/api/v1/search/?s={}",
                crate::engine::util::encode(query)
            ),
            true,
        ),
        // duckduckgo is the default backend
        _ => (
            format!(
                "https://duckduckgo.com/ac/?type=list&q={}",
                crate::engine::util::encode(query)
            ),
            true,
        ),
    };
    let _ = is_json;
    let request = EngineRequest::get(url).header("Accept", "application/json");
    let timeout = std::time::Duration::from_secs_f64(settings.search.timeout_limit.clamp(1.0, 3.0));
    match network.fetch(request, timeout, None, None).await {
        Ok(response) => parse_suggestions(backend, &response),
        Err(error) => {
            tracing::debug!(backend, %error, "autocomplete backend failed");
            Vec::new()
        }
    }
}

/// Extract the suggestions from a backend response, keeping the backend's own
/// relevance order: the ranking is the entire point of an autocomplete
/// suggestion list, so results are only de-duplicated, never re-sorted.
pub fn parse_suggestions(backend: &str, response: &EngineResponse) -> Vec<String> {
    let Ok(json) = response.json() else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut suggestions = Vec::new();
    let mut push = |text: String| {
        if seen.insert(text.clone()) {
            suggestions.push(text);
        }
    };
    match backend {
        "wikipedia" => {
            if let Some(items) = json_path_array(&json, 1) {
                for item in items {
                    if let Some(text) = item.as_str() {
                        push(text.to_string());
                    }
                }
            }
        }
        "mwmbl" => {
            if let Some(items) = json.as_array() {
                for item in items.iter().take(10) {
                    let title = json_str(item, "title.0.value");
                    if !title.is_empty() {
                        push(title);
                    }
                }
            }
        }
        // duckduckgo: [{"phrase": "..."}], array responses: ["query", ["...", ...]]
        _ => {
            if let Some(items) = json.as_array() {
                for item in items {
                    if let Some(text) = item.as_str() {
                        push(text.to_string());
                    } else {
                        let phrase = json_str(item, "phrase");
                        if !phrase.is_empty() {
                            push(phrase);
                        }
                    }
                }
            }
            if let Some(items) = json_path_array(&json, 1) {
                for item in items {
                    if let Some(text) = item.as_str() {
                        push(text.to_string());
                    }
                }
            }
        }
    }
    drop(push);
    suggestions.truncate(10);
    suggestions
}

fn json_path_array(value: &serde_json::Value, index: usize) -> Option<&[serde_json::Value]> {
    value.as_array()?.get(index)?.as_array().map(Vec::as_slice)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineResponse;

    fn response(body: &str) -> EngineResponse {
        EngineResponse::new(200, "https://example.com/", body.as_bytes().to_vec())
    }

    #[test]
    fn duckduckgo_suggestions_are_parsed() {
        let body = r#"[{"phrase":"rust"},{"phrase":"rust book"}]"#;
        assert_eq!(
            parse_suggestions("duckduckgo", &response(body)),
            vec!["rust".to_string(), "rust book".to_string()]
        );
    }

    #[test]
    fn wikipedia_suggestions_are_parsed() {
        let body = r#"["rust",["Rust","Rust (programming language)"]]"#;
        assert_eq!(
            parse_suggestions("wikipedia", &response(body)),
            vec![
                "Rust".to_string(),
                "Rust (programming language)".to_string()
            ]
        );
    }

    #[test]
    fn broken_json_yields_no_suggestions() {
        assert!(parse_suggestions("duckduckgo", &response("<html>")).is_empty());
    }

    #[test]
    fn suggestion_order_follows_backend_ranking_not_alphabetical_order() {
        // The backend ranks "zebra" above "apple"; a `BTreeSet` would have
        // alphabetized the pair and silently thrown that ranking away.
        let body = r#"[{"phrase":"zebra"},{"phrase":"apple"}]"#;
        assert_eq!(
            parse_suggestions("duckduckgo", &response(body)),
            vec!["zebra".to_string(), "apple".to_string()]
        );
    }

    #[test]
    fn duplicate_suggestions_are_removed_without_reordering() {
        let body = r#"[{"phrase":"rust"},{"phrase":"ruby"},{"phrase":"rust"}]"#;
        assert_eq!(
            parse_suggestions("duckduckgo", &response(body)),
            vec!["rust".to_string(), "ruby".to_string()]
        );
    }
}
