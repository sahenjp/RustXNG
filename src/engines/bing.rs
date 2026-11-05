//! Bing engine, ported from `searx/engines/bing.py`.

use std::sync::Arc;

use crate::engine::util::*;
use crate::engine::{
    Engine, EngineDefaults, EngineError, EngineItem, EngineMeta, EngineRequest, EngineResponse,
};
use crate::query::{Query, TimeRange};
use crate::settings::EngineConfig;

const DEFAULTS: EngineDefaults = EngineDefaults {
    categories: &["general", "web"],
    shortcut: "bi",
    paging: true,
    time_range_support: true,
    safesearch: true,
    language_support: true,
    weight: 1.0,
    timeout: 4.0,
};

const BASE_URL: &str = "https://www.bing.com/search";

/// Bing web search.
pub struct Bing {
    meta: EngineMeta,
    market: Option<String>,
}

impl Bing {
    pub fn from_config(config: &EngineConfig) -> Result<Arc<Self>, String> {
        let market = config
            .opt_str("mkt")
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(market) = market {
            let parts: Vec<_> = market.split('-').collect();
            if parts.len() != 2
                || parts
                    .iter()
                    .any(|part| part.len() != 2 || !part.bytes().all(|c| c.is_ascii_alphabetic()))
            {
                return Err("bing mkt must be a language-region pair such as en-GB".into());
            }
        }
        Ok(Arc::new(Self {
            meta: EngineMeta::from_config(config, DEFAULTS),
            market: market.map(str::to_string),
        }))
    }
}

impl Engine for Bing {
    fn meta(&self) -> &EngineMeta {
        &self.meta
    }

    fn build_request(&self, query: &Query, pageno: u32) -> Result<EngineRequest, EngineError> {
        let locale = self.market.as_deref().unwrap_or(&query.locale);
        let language = locale_language(locale);
        let region = locale_region(locale);
        let mut pairs: Vec<(&str, String)> = vec![
            ("q", query.query_text()),
            (
                "adlt",
                match query.safesearch {
                    0 => "off",
                    1 => "moderate",
                    _ => "strict",
                }
                .to_string(),
            ),
            (
                "first",
                ((u64::from(pageno.max(1)) - 1) * 10 + 1).to_string(),
            ),
            ("count", "10".to_string()),
        ];
        if let Some(market) = &self.market {
            pairs.push(("mkt", market.clone()));
        }
        if language != "all" {
            pairs.push(("setlang", language.clone()));
        }
        if region != "all" && !matches!(region.as_str(), "us" | "cn" | "ru") {
            pairs.push(("cc", region));
        }
        if let Some(range) = query.time_range {
            let value = match range {
                TimeRange::Day => "ez1",
                TimeRange::Week => "ez2",
                TimeRange::Month => "ez3",
                TimeRange::Year => "ez5",
            };
            pairs.push(("filters", format!("ex1%3a%22{value}%22")));
        }
        Ok(
            EngineRequest::get(format!("{BASE_URL}?{}", query_string(&pairs)))
                .header("Accept-Language", &format!("{language};q=0.9")),
        )
    }

    fn parse(
        &self,
        response: &EngineResponse,
        _query: &Query,
    ) -> Result<Vec<EngineItem>, EngineError> {
        let html = parse_html(&response.text());
        let mut results = Vec::new();
        for element in select_elements(&html, "li.b_algo")? {
            let Some(link) = child(element, "h2 a") else {
                continue;
            };
            let Some(href) = attr(link, "href") else {
                continue;
            };
            let Some(url) = absolute_url(&href, "https://www.bing.com/") else {
                continue;
            };
            let title = text(link);
            if title.is_empty() {
                continue;
            }
            let content = text_of(element, "div.b_caption p")
                .or_else(|| text_of(element, "p"))
                .or_else(|| text_of(element, "div.b_snippet"))
                .unwrap_or_default();
            let mut builder = ResultBuilder::new(url, title).content(content);
            if let Some(date) = text_of(element, "span.news_dt") {
                builder = builder.published_date(date);
            }
            results.push(builder.build());
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{engine::EngineRegistry, settings::Settings};

    fn config(name: &str, shortcut: &str, market: &str) -> EngineConfig {
        let mut config = EngineConfig::new(name, "bing", shortcut, &["general"]);
        config
            .extra
            .insert("mkt".into(), toml::Value::String(market.into()));
        config
    }

    #[test]
    fn configured_market_overrides_request_locale() {
        let engine = Bing::from_config(&config("bing UK", "buk", "en-GB")).unwrap();
        let mut query = Query::parse("rust", &EngineRegistry::default(), &[]);
        query.locale = "ja-JP".into();
        let request = engine.build_request(&query, 1).unwrap();
        let url = url::Url::parse(&request.url).unwrap();
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(pairs.get("mkt").map(String::as_str), Some("en-GB"));
        assert_eq!(pairs.get("setlang").map(String::as_str), Some("en"));
        assert_eq!(pairs.get("cc").map(String::as_str), Some("gb"));
        assert!(engine.build_request(&query, u32::MAX).is_ok());
        assert!(Bing::from_config(&config("bad", "bad", "en-GB&x=y")).is_err());
    }

    #[test]
    fn registry_keeps_independent_entries_with_the_same_engine_type() {
        let settings = Settings {
            engines: vec![
                config("bing UK", "buk", "en-GB"),
                config("bing Germany", "bde", "de-DE"),
                config("bing Japan", "bjp", "ja-JP"),
            ],
            ..Default::default()
        };
        let registry = EngineRegistry::from_settings(&settings).unwrap();
        assert_eq!(registry.len(), 3);
        for (shortcut, name) in [
            ("buk", "bing UK"),
            ("bde", "bing Germany"),
            ("bjp", "bing Japan"),
        ] {
            let query = Query::parse(&format!("!{shortcut} rust"), &registry, &[]);
            assert_eq!(query.engine_ref_names(), vec![name.to_string()]);
        }
    }

    #[test]
    fn configured_markets_are_opt_in_in_the_default_registry() {
        let registry = EngineRegistry::from_settings(&Settings::defaults()).unwrap();
        assert!(registry.get("bing").is_none());
        assert!(registry.by_shortcut("buk").is_none());
        assert!(registry.by_shortcut("bde").is_none());
        assert!(registry.by_shortcut("bjp").is_none());
    }
}
