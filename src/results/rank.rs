//! Ranking and policy for merged results: query-term relevance, source
//! authority, page-language preference and de-duplication grouping.
//!
//! Everything here is a pure function over a result (or a slice of results):
//! no I/O, no configuration lookup, no container state. The scoring weights and
//! the decision of when to apply them stay in ResultContainer::close, which is
//! the only caller.

use std::collections::HashMap;

use super::{SearchResult, domain_matches, host_of};
use crate::query::query_language;

/// Lowercase query terms (latin words of 3+, plus CJK bigrams) used by
/// [`relevance`].
pub fn query_terms(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut terms: Vec<String> = lower
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.chars().count() >= 3)
        .map(str::to_string)
        .collect();
    let cjk: Vec<char> = lower
        .chars()
        .filter(|character| !character.is_ascii() && character.is_alphanumeric())
        .collect();
    for window in cjk.windows(2) {
        terms.push(window.iter().collect());
    }
    terms.sort();
    terms.dedup();
    terms
}

/// How many of the query terms appear in the URL + title (0.0 ..= 1.0).
///
/// A query term in the path/slug (e.g. `functions-json` for "json functions")
/// is a strong signal; the helper counts distinct terms present in the
/// lowercased URL and title.
pub(crate) fn relevance(url: &str, title: &str, content: &str, terms: &[String]) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    // A bare homepage is rarely the answer to a specific query; its path has no
    // depth and it repeats none of the query in its URL. Deep pages win ties.
    let path_depth = {
        let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
        rest.split(['?', '#'])
            .next()
            .unwrap_or(rest)
            .split('/')
            .skip(1)
            .filter(|segment| !segment.is_empty())
            .count()
    };
    let depth_bonus = if path_depth == 0 { 0.0 } else { 0.35 };
    // Percent-decode so CJK paths (ja.wikipedia.org/wiki/%E5%AF%8C%E5%A3%AB...)
    // match the query's character bigrams; otherwise Japanese gold URLs score 0.
    let decoded_url = percent_encoding::percent_decode_str(url)
        .decode_utf8_lossy()
        .to_lowercase();
    let heading = format!("{decoded_url} {}", title.to_lowercase());
    let heading_hits = terms
        .iter()
        .filter(|term| heading.contains(term.as_str()))
        .count();
    let heading_coverage = heading_hits as f64 / terms.len() as f64;
    // The snippet is what the agent actually reads. A page whose text talks
    // about the query is a better answer than a page that only mentions it in
    // the URL, so blend the body coverage in at a lower weight.
    let body: String = content.chars().take(600).collect::<String>().to_lowercase();
    let body_hits = terms
        .iter()
        .filter(|term| body.contains(term.as_str()))
        .count();
    let body_coverage = body_hits as f64 / terms.len() as f64;
    // An identifier in the query (rfc 9110, oauth 6749, python 3.12) is the most
    // discriminating token there is: when the URL carries it, the page really is
    // the one being asked for.
    let id_bonus = if terms
        .iter()
        .filter(|term| term.chars().any(|c| c.is_ascii_digit()) && term.len() >= 3)
        .any(|term| decoded_url.contains(term.as_str()))
    {
        0.25
    } else {
        0.0
    };
    (0.62 * heading_coverage + 0.30 * body_coverage + depth_bonus + id_bonus).min(1.0)
}

/// Relevance of one result to the query terms, for callers outside this
/// module (the site-scoped second pass picks a site's best page with this).
pub fn relevance_score(result: &SearchResult, terms: &[String]) -> f64 {
    relevance(&result.url, &result.title, &result.content, terms)
}

/// How much of the query the title and snippet of a result cover
/// (0.0 ..= 1.0).
///
/// This is the text-only companion of [relevance]: it ignores the URL
/// slug, path depth and identifier bonuses, so a page whose *title* answers
/// the query is lifted even when its address does not repeat the terms.
/// The title is the page's own label and weighs more than the snippet.
pub(crate) fn text_match(title: &str, content: &str, terms: &[String]) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let title = title.to_lowercase();
    let snippet: String = content.chars().take(600).collect::<String>().to_lowercase();
    let title_hits = terms
        .iter()
        .filter(|term| title.contains(term.as_str()))
        .count();
    let snippet_hits = terms
        .iter()
        .filter(|term| snippet.contains(term.as_str()))
        .count();
    let title_coverage = title_hits as f64 / terms.len() as f64;
    let snippet_coverage = snippet_hits as f64 / terms.len() as f64;
    0.7 * title_coverage + 0.3 * snippet_coverage
}

/// Agreement of independent sources, in 0.0 ..= 1.0.
///
/// One source is 0.0, two are 0.5, three are 0.67: the value grows with every
/// additional engine but never reaches 1.0, so engine count cannot outweigh
/// the position fusion on its own.
pub(crate) fn consensus(source_count: usize) -> f64 {
    if source_count <= 1 {
        return 0.0;
    }
    (source_count as f64 - 1.0) / source_count as f64
}

/// A brand/site the user names in the query, with its canonical hosts.
struct Authority {
    /// Query substrings that name the site (lowercase).
    names: &'static [&'static str],
    /// Hosts that are the real, canonical source.
    canonical: &'static [&'static str],
}

/// Sites people name explicitly in a search query. Naming one is a request for
/// that site's own documentation, not for a mirror or a tutorial about it.
///
/// The list stays small and conservative: each entry is a site whose name is
/// unambiguous enough to act on.
const AUTHORITIES: &[Authority] = &[
    Authority {
        names: &["arxiv"],
        canonical: &["arxiv.org"],
    },
    Authority {
        names: &["mdn"],
        canonical: &["developer.mozilla.org"],
    },
    Authority {
        names: &["python", "python3"],
        canonical: &["docs.python.org", "python.org"],
    },
    Authority {
        names: &["python japan"],
        canonical: &["python.jp", "python.org"],
    },
    Authority {
        names: &["rust", "rustlang"],
        canonical: &["doc.rust-lang.org", "rust-lang.org"],
    },
    Authority {
        names: &["rust japan", "rust-jp"],
        canonical: &["rust-jp.rs", "doc.rust-lang.org"],
    },
    Authority {
        names: &["postgres", "postgresql"],
        canonical: &["postgresql.org"],
    },
    Authority {
        names: &["docker"],
        canonical: &["docs.docker.com", "docker.com"],
    },
    Authority {
        names: &["docker hub", "dockerhub"],
        canonical: &["hub.docker.com"],
    },
    Authority {
        names: &["linux", "kernel"],
        canonical: &["docs.kernel.org", "kernel.org"],
    },
    Authority {
        names: &["numpy"],
        canonical: &["numpy.org"],
    },
    Authority {
        names: &["react"],
        canonical: &["react.dev", "reactjs.org", "ja.react.dev"],
    },
    Authority {
        names: &["npm"],
        canonical: &["npmjs.com"],
    },
    Authority {
        names: &["crates", "crates.io"],
        canonical: &["crates.io", "docs.rs"],
    },
    Authority {
        names: &["wikipedia"],
        canonical: &["wikipedia.org"],
    },
    Authority {
        names: &["kubernetes", "k8s"],
        canonical: &["kubernetes.io"],
    },
    Authority {
        names: &["git"],
        canonical: &["git-scm.com"],
    },
    Authority {
        names: &["nginx"],
        canonical: &["nginx.org", "docs.nginx.com"],
    },
    Authority {
        names: &["fastapi"],
        canonical: &["fastapi.tiangolo.com"],
    },
    Authority {
        names: &["django"],
        canonical: &["docs.djangoproject.com", "djangoproject.com"],
    },
    Authority {
        names: &["flask"],
        canonical: &["flask.palletsprojects.com"],
    },
    Authority {
        names: &["rfc"],
        canonical: &["rfc-editor.org", "ietf.org"],
    },
    Authority {
        names: &["json schema"],
        canonical: &["json-schema.org"],
    },
    Authority {
        names: &["oauth"],
        canonical: &["rfc-editor.org", "oauth.net"],
    },
    Authority {
        names: &["man page", "man7"],
        canonical: &["man7.org", "linux.die.net"],
    },
    Authority {
        names: &["tokio"],
        canonical: &["tokio.rs"],
    },
    Authority {
        names: &["serde"],
        canonical: &["serde.rs", "crates.io", "docs.rs"],
    },
    Authority {
        names: &["sqlite"],
        canonical: &["sqlite.org"],
    },
    Authority {
        names: &["redis"],
        canonical: &["redis.io"],
    },
    Authority {
        names: &["golang"],
        canonical: &["go.dev", "pkg.go.dev"],
    },
    Authority {
        names: &["nodejs", "node.js"],
        canonical: &["nodejs.org"],
    },
    Authority {
        names: &["typescript"],
        canonical: &["typescriptlang.org"],
    },
    Authority {
        names: &["tailwind"],
        canonical: &["tailwindcss.com"],
    },
    Authority {
        names: &["vite"],
        canonical: &["vitejs.dev", "vite.dev"],
    },
    Authority {
        names: &["pytorch"],
        canonical: &["pytorch.org"],
    },
    Authority {
        names: &["tensorflow"],
        canonical: &["tensorflow.org"],
    },
    Authority {
        names: &["huggingface", "hugging face"],
        canonical: &["huggingface.co"],
    },
    Authority {
        names: &["aws", "ec2"],
        canonical: &["docs.aws.amazon.com", "aws.amazon.com"],
    },
    Authority {
        names: &["google cloud", "gcp"],
        canonical: &["cloud.google.com"],
    },
    Authority {
        names: &["cppreference"],
        canonical: &["en.cppreference.com", "cppreference.com"],
    },
    Authority {
        names: &["arch wiki", "archlinux"],
        canonical: &["wiki.archlinux.org", "archlinux.org"],
    },
    Authority {
        names: &["whatwg"],
        canonical: &["whatwg.org"],
    },
    Authority {
        names: &["jquery"],
        canonical: &["jquery.com"],
    },
    Authority {
        names: &["気象庁"],
        canonical: &["jma.go.jp"],
    },
    Authority {
        names: &["国立国会図書館"],
        canonical: &["ndlsearch.ndl.go.jp", "ndl.go.jp"],
    },
    Authority {
        names: &["gotokyo"],
        canonical: &["gotokyo.org"],
    },
    Authority {
        names: &["jr東日本", "jreast"],
        canonical: &["jreast.co.jp"],
    },
    Authority {
        names: &["e-gov", "法令"],
        canonical: &["elaws.e-gov.go.jp", "e-gov.go.jp"],
    },
];

/// Hosts that host a stale copy of another site's documentation. They are
/// mirrors of the original and must never outrank it.
const MIRRORS: &[&str] = &[
    "docker-docs.uclv.cu",
    "docker-docs.netlify.app",
    "docs.docker.com.cn",
    "postgrespro.com",
    "readthedocs.org",
    "gitmemory.com",
    "devdocs.io",
    "manpages.ubuntu.com",
];

/// Penalty for a Wikipedia edition that does not match the query language.
///
/// A Czech or Gan Wikipedia article is almost never the answer to an English,
/// Japanese or Korean query; those editions only add noise to the merged
/// results.
pub(crate) fn wikipedia_language_penalty(query: &str, url: &str) -> f64 {
    let Some(host) = host_of(url) else { return 0.0 };
    let Some(subdomain) = host.strip_suffix(".wikipedia.org") else {
        return 0.0;
    };
    if subdomain.contains('.') {
        return 0.0; // e.g. zh-min-nan.wikipedia.org counts as its own lang
    }
    let matches = match query_language(query) {
        "ja" => matches!(subdomain, "ja"),
        "ko" => matches!(subdomain, "ko"),
        _ => matches!(subdomain, "en" | "simple"),
    };
    if matches { 0.0 } else { -0.45 }
}

/// Language codes that documentation sites put in their URLs.
const PAGE_LANGUAGES: &[&str] = &[
    "ar", "cs", "de", "en", "es", "fa", "fr", "he", "hu", "id", "it", "ja", "ko", "nl", "pl", "pt",
    "ru", "sv", "th", "tr", "uk", "vi", "zh",
];

/// The language a URL marks its page as being in, if any: a leading path
/// segment (`/ja/`, `/ja-jp/`, `/zh_CN/`), a `translations/<lang>` segment, or
/// a language subdomain (`ja.react.dev`). Wikipedia is handled on its own.
pub fn page_language(url: &str) -> Option<&'static str> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if host.ends_with(".wikipedia.org") {
        return None;
    }
    let code = |segment: &str| -> Option<&'static str> {
        let lower = segment.to_ascii_lowercase();
        let (language, region) = match lower.split_once(['-', '_']) {
            Some((language, region)) => (language.to_string(), Some(region.to_string())),
            None => (lower, None),
        };
        // A region or script is short ("jp", "hans"); anything longer is a word.
        if region
            .as_ref()
            .is_some_and(|region| !(2..=4).contains(&region.len()))
        {
            return None;
        }
        PAGE_LANGUAGES
            .iter()
            .find(|known| **known == language)
            .copied()
    };
    let segments: Vec<&str> = parsed.path_segments()?.filter(|s| !s.is_empty()).collect();
    if let Some(found) = segments.first().and_then(|segment| code(segment)) {
        return Some(found);
    }
    if let Some(index) = segments
        .iter()
        .position(|segment| *segment == "translations")
    {
        if let Some(found) = segments.get(index + 1).and_then(|segment| code(segment)) {
            return Some(found);
        }
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() >= 3 {
        // "id." and "it." are more often services than languages.
        if let Some(found) = code(labels[0]).filter(|code| !matches!(*code, "id" | "it")) {
            return Some(found);
        }
    }
    None
}

/// Score adjustment for a page whose URL names its language: a page in the
/// query's language is lifted a little, one in a third language pushed down.
/// English pages are neutral (the usual fallback for documentation).
pub(crate) fn page_language_adjustment(query: &str, url: &str) -> f64 {
    let Some(language) = page_language(url) else {
        return 0.0;
    };
    let wanted = query_language(query);
    if language == wanted {
        if wanted == "en" { 0.0 } else { 0.3 }
    } else if language == "en" {
        0.0
    } else {
        -0.45
    }
}

/// Match a canonical host.
///
/// Most sites run separate services on subdomains (\`users.rust-lang.org\` is the
/// forum, not the documentation), so only the bare host and its \`www.\` variant
/// match. Wikipedia is the exception: it is partitioned by language and every
/// edition is genuinely the same encyclopaedia.
fn canonical_matches(host: &str, domain: &str) -> bool {
    if host == domain || host == format!("www.{domain}") {
        return true;
    }
    if host.strip_prefix("www.") == Some(domain) {
        return true;
    }
    const WHOLE_SITE: &[&str] = &["wikipedia.org"];
    WHOLE_SITE
        .iter()
        .any(|site| domain == *site && domain_matches(host, site))
}

/// The canonical host the query asks for, when it names a known site.
///
/// \`"python dict get method"\` -> \`Some("docs.python.org")\`. Used to run a
/// site-scoped second pass when the site's own page did not surface.
pub fn canonical_host_for_query(query: &str) -> Option<&'static str> {
    let lower = query.to_lowercase();
    let words: Vec<&str> = lower
        .split(|character: char| {
            !character.is_alphanumeric() && character != '.' && character != '-'
        })
        .collect();
    for authority in AUTHORITIES {
        let named = authority.names.iter().any(|name| {
            words.iter().any(|word| word == name)
                || (name.chars().count() >= 6 && lower.contains(name))
        });
        if named {
            // Prefer the most specific canonical host (the documentation one).
            return authority
                .canonical
                .iter()
                .max_by_key(|domain| domain.split('.').count())
                .copied();
        }
    }
    None
}

/// Pick the canonical host this query is really asking for.
///
/// A site can name several hosts, and the right one depends on the wording:
/// "docker hub" wants hub.docker.com while "docker run reference" wants
/// docs.docker.com. Score each host by how many of its distinctive labels
/// appear in the query, then prefer the more specific host on a tie.
fn best_canonical_for(query: &str, host: &str, authority: &Authority) -> Option<bool> {
    let words: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect();
    let score = |domain: &str| -> (usize, usize) {
        let labels: Vec<&str> = domain.split('.').collect();
        let distinctive = labels[..labels.len().saturating_sub(2)]
            .iter()
            .filter(|label| words.iter().any(|word| word == *label))
            .count();
        (distinctive, labels.len())
    };
    let global = authority.canonical.iter().map(|d| score(d)).max()?;
    let matching = authority
        .canonical
        .iter()
        .filter(|domain| canonical_matches(host, domain))
        .map(|domain| score(domain))
        .max()?;
    Some(matching == global)
}

/// How authoritative a result is for this query, in 0.0 ..= 1.0.
///
/// A named site's canonical hosts score 1.0, its mirror copies score 0.0 and
/// everything else sits in the middle. This is what separates "the official
/// page" from "a tutorial that quotes it".
pub(crate) fn authority(query: &str, url: &str) -> f64 {
    let lower = query.to_lowercase();
    let host = match host_of(url) {
        Some(host) => host,
        None => return 0.5,
    };
    let words: Vec<&str> = lower
        .split(|character: char| {
            !character.is_alphanumeric() && character != '.' && character != '-'
        })
        .collect();
    // Several sites can be named at once ("docker hub" also names "docker"), so
    // take the most specific name that matched: the longer phrase wins.
    let mut matched: Option<(&Authority, usize)> = None;
    for authority in AUTHORITIES {
        // A short name only matches a whole word ("git" must not match "github");
        // a distinctive name may appear inside a compound ("postgresql").
        let score = authority
            .names
            .iter()
            .filter(|name| {
                words.iter().any(|word| word == *name)
                    || (name.chars().count() >= 6 && lower.contains(*name))
            })
            .map(|name| name.chars().count())
            .max();
        if let Some(score) = score {
            if matched.is_none_or(|(_, best)| score > best) {
                matched = Some((authority, score));
            }
        }
    }
    {
        let Some((authority, _)) = matched else {
            if MIRRORS.iter().any(|mirror| domain_matches(&host, mirror)) {
                return 0.2;
            }
            return 0.5;
        };
        if let Some(best) = best_canonical_for(query, &host, authority) {
            return if best { 1.0 } else { 0.85 };
        }
        if MIRRORS.iter().any(|mirror| domain_matches(&host, mirror)) {
            return 0.0;
        }
        0.4
    }
}

pub(crate) fn order_results(results: &[SearchResult]) -> Vec<SearchResult> {
    let mut sorted: Vec<SearchResult> = results.to_vec();
    sorted.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    const MAX_COUNT: i32 = 8;
    const MAX_DISTANCE: usize = 20;

    // 並べ替えはまず添字だけで行う。同一カテゴリの挿入は `Vec::insert` の
    // memmove を伴い、SearchResult を直接動かすとエンジンが返す件数の2乗に
    // 比例するメモリ帯域を使う。usize の並べ替えなら同じ回数でも定数分の
    // 1/30 ほどで済み、最後に一度だけ実体を並べ替えられる。
    let mut grouped: Vec<usize> = Vec::with_capacity(sorted.len());
    let mut positions: HashMap<String, GroupState> = HashMap::new();

    for (sequence, result) in sorted.iter().enumerate() {
        let category = format!(
            "{}:{}:{}",
            result.category,
            result.template.as_str(),
            if result.thumbnail.is_some() || result.img_src.is_some() {
                "img"
            } else {
                ""
            }
        );
        let group = positions.get(&category).copied();
        match group {
            Some(state)
                if state.count > 0 && grouped.len().saturating_sub(state.index) < MAX_DISTANCE =>
            {
                let index = state.index;
                grouped.insert(index, sequence);
                for state in positions.values_mut() {
                    if state.index >= index {
                        state.index += 1;
                    }
                }
                if let Some(state) = positions.get_mut(&category) {
                    state.count -= 1;
                }
            }
            _ => {
                grouped.push(sequence);
                positions.insert(
                    category,
                    GroupState {
                        index: grouped.len(),
                        count: MAX_COUNT,
                    },
                );
            }
        }
    }

    // `grouped` は 0..sorted.len() の順列なので、各実体は一度だけ取り出せる。
    let mut slots: Vec<Option<SearchResult>> = sorted.into_iter().map(Some).collect();
    grouped
        .into_iter()
        .map(|index| slots[index].take().expect("each result is placed once"))
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct GroupState {
    index: usize,
    count: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_results_keeps_one_category_together() {
        // Upstream get_ordered_results: results of one category are spread with
        // at most eight neighbours between them, and nothing is dropped.
        let mut results = Vec::new();
        for index in 0..4 {
            let mut image = SearchResult::new(format!("https://img.example.com/{index}"), "img");
            image.category = "images".into();
            image.template = super::super::Template::Images;
            image.score = 1.0;
            results.push(image);
            let mut web = SearchResult::new(format!("https://web.example.com/{index}"), "web");
            web.category = "general".into();
            web.score = 10.0 - index as f64;
            results.push(web);
        }
        let ordered = order_results(&results);
        assert_eq!(ordered.len(), results.len());
        // The higher scored web results keep their order at the front ...
        assert_eq!(
            ordered[..4]
                .iter()
                .map(|r| r.url.as_str())
                .collect::<Vec<_>>(),
            [
                "https://web.example.com/0",
                "https://web.example.com/1",
                "https://web.example.com/2",
                "https://web.example.com/3",
            ]
        );
        // ... and the lower scored image group follows as one block.
        assert!(
            ordered[4..]
                .iter()
                .all(|r| r.url.contains("img.example.com"))
        );
    }

    #[test]
    fn page_languages_are_read_from_urls() {
        assert_eq!(
            page_language("https://docs.python.org/ja/3/library/"),
            Some("ja")
        );
        assert_eq!(
            page_language("https://developer.mozilla.org/en-US/docs/Web"),
            Some("en")
        );
        assert_eq!(
            page_language("https://learn.microsoft.com/ja-jp/azure/"),
            Some("ja")
        );
        assert_eq!(page_language("https://ja.react.dev/learn"), Some("ja"));
        assert_eq!(
            page_language("https://www.kernel.org/doc/html/latest/translations/zh_CN/index.html"),
            Some("zh")
        );
        assert_eq!(page_language("https://docs.python.org/3/library/"), None);
        assert_eq!(page_language("https://example.org/de-facto-standard"), None);
        assert_eq!(page_language("https://id.atlassian.com/login"), None);
        assert_eq!(page_language("https://ja.wikipedia.org/wiki/X"), None);
    }

    #[test]
    fn pages_in_the_query_language_are_preferred() {
        let ja = "https://developer.mozilla.org/ja/docs/Web/JavaScript";
        let en = "https://developer.mozilla.org/en-US/docs/Web/JavaScript";
        let fr = "https://developer.mozilla.org/fr/docs/Web/JavaScript";
        assert!(page_language_adjustment("mdn javascript 入門", ja) > 0.0);
        assert_eq!(page_language_adjustment("mdn javascript 入門", en), 0.0);
        assert!(page_language_adjustment("mdn javascript intro", ja) < 0.0);
        assert!(page_language_adjustment("mdn javascript intro", fr) < 0.0);
        assert_eq!(page_language_adjustment("mdn javascript intro", en), 0.0);
        // Korean query: /ko/ pages lift, English stays neutral, others sink
        let ko = "https://developer.mozilla.org/ko/docs/Web/JavaScript";
        assert!(page_language_adjustment("mdn javascript 입문", ko) > 0.0);
        assert_eq!(page_language_adjustment("mdn javascript 입문", en), 0.0);
        assert!(page_language_adjustment("mdn javascript 입문", ja) < 0.0);
    }

    #[test]
    fn canonical_subsites_do_not_lend_authority_to_neighbours() {
        // docs.rust-lang.org is exact: the forum next door is not "the docs"
        assert_eq!(
            authority("rust traits", "https://doc.rust-lang.org/book/"),
            1.0
        );
        assert!(authority("rust traits", "https://users.rust-lang.org/t/x") < 1.0);
        // a whole-site canonical still covers its subdomains
        assert_eq!(
            authority("numpy indexing", "https://numpy.org/doc/stable/"),
            1.0
        );
        assert_eq!(
            authority("wikipedia tokyo", "https://ja.wikipedia.org/wiki/東京"),
            1.0
        );
    }

    #[test]
    fn wrong_language_wikipedia_is_penalised() {
        // English query: en/simple fine, cs/gan are noise
        assert_eq!(
            wikipedia_language_penalty("tokyo wikipedia", "https://en.wikipedia.org/wiki/Tokyo"),
            0.0
        );
        assert!(
            wikipedia_language_penalty("tokyo wikipedia", "https://cs.wikipedia.org/wiki/Tokyo")
                < 0.0
        );
        assert!(
            wikipedia_language_penalty("tokyo wikipedia", "https://gan.wikipedia.org/wiki/Tokyo")
                < 0.0
        );
        // Japanese query: ja fine, en is noise
        assert_eq!(
            wikipedia_language_penalty("富士山 wikipedia", "https://ja.wikipedia.org/wiki/富士山"),
            0.0
        );
        assert!(
            wikipedia_language_penalty(
                "富士山 wikipedia",
                "https://en.wikipedia.org/wiki/Mount_Fuji"
            ) < 0.0
        );
        // Korean query: ko fine, en and ja are noise
        assert_eq!(
            wikipedia_language_penalty("서울 wikipedia", "https://ko.wikipedia.org/wiki/서울"),
            0.0
        );
        assert!(
            wikipedia_language_penalty("서울 wikipedia", "https://en.wikipedia.org/wiki/Seoul")
                < 0.0
        );
        assert!(
            wikipedia_language_penalty("서울 wikipedia", "https://ja.wikipedia.org/wiki/ソウル")
                < 0.0
        );
        // non-wikipedia hosts are untouched
        assert_eq!(
            wikipedia_language_penalty("tokyo", "https://tokyo.rs/"),
            0.0
        );
    }

    #[test]
    fn canonical_host_for_query_finds_the_named_site() {
        assert_eq!(
            canonical_host_for_query("python dict get method"),
            Some("docs.python.org")
        );
        assert_eq!(
            canonical_host_for_query("rust traits"),
            Some("doc.rust-lang.org")
        );
        assert_eq!(
            canonical_host_for_query("docker compose reference"),
            Some("docs.docker.com")
        );
        assert_eq!(canonical_host_for_query("some random topic"), None);
    }

    #[test]
    fn title_and_snippet_matches_are_counted_separately_from_the_url() {
        let terms = query_terms("async runtime tutorial");
        let titled = text_match("Async runtime tutorial", "how to use the runtime", &terms);
        let snippet_only = text_match(
            "Some page",
            "an async runtime tutorial for beginners",
            &terms,
        );
        let missing = text_match("Unrelated", "nothing here", &terms);
        assert!(titled > snippet_only, "{titled} vs {snippet_only}");
        assert!(snippet_only > missing);
        assert_eq!(missing, 0.0);
        assert_eq!(text_match("anything", "anything", &[]), 0.0);
    }

    #[test]
    fn consensus_grows_with_the_number_of_sources() {
        assert_eq!(consensus(0), 0.0);
        assert_eq!(consensus(1), 0.0);
        assert_eq!(consensus(2), 0.5);
        assert!(consensus(3) > consensus(2));
        assert!(consensus(10) < 1.0, "it never reaches one");
    }

    #[test]
    fn identifiers_in_the_query_boost_the_matching_url() {
        let terms = query_terms("http semantics rfc 9110");
        let exact = relevance(
            "https://www.rfc-editor.org/rfc/rfc9110",
            "RFC 9110: HTTP Semantics",
            "",
            &terms,
        );
        let other = relevance(
            "https://www.rfc-editor.org/rfc/rfc2616",
            "RFC 2616: HTTP/1.1",
            "",
            &terms,
        );
        assert!(exact > other, "{exact} should beat {other}");
    }

    #[test]
    fn the_canonical_host_follows_the_query_wording() {
        // "docker hub" asks for the hub, "docker run reference" for the docs
        assert_eq!(
            authority("docker hub", "https://hub.docker.com/_/nginx"),
            1.0
        );
        assert!(authority("docker hub", "https://docs.docker.com/engine/") < 1.0);
        assert_eq!(
            authority("docker run reference", "https://docs.docker.com/reference/"),
            1.0
        );
        assert!(authority("docker run reference", "https://hub.docker.com/_/nginx") < 1.0);
    }

    #[test]
    fn authority_lifts_canonical_and_demotes_mirrors() {
        // naming arxiv: the canonical host wins, wikipedia loses
        assert_eq!(
            authority(
                "attention is all you need arxiv",
                "https://arxiv.org/abs/1706.03762"
            ),
            1.0
        );
        assert!(
            authority(
                "attention is all you need arxiv",
                "https://en.wikipedia.org/wiki/Attention"
            ) < 0.5
        );
        // naming docker: official docs win, a mirror is pushed to the floor
        assert_eq!(
            authority(
                "docker compose file reference",
                "https://docs.docker.com/reference/compose-file/"
            ),
            1.0
        );
        assert_eq!(
            authority(
                "docker compose file reference",
                "https://docker-docs.uclv.cu/compose/compose-file/"
            ),
            0.0
        );
        // a named site in the list is lifted to its canonical host
        assert_eq!(authority("tokio async runtime", "https://tokio.rs/"), 1.0);
        // no listed site named: everything sits in the middle
        assert_eq!(authority("some random topic", "https://example.com/"), 0.5);
        // a mirror of an unnamed site is still demoted
        assert_eq!(authority("some docs", "https://docker-docs.uclv.cu/x"), 0.2);
    }
}
