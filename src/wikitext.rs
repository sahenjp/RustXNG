//! A small wikitext reader: the readable opening of an article.
//!
//! The Wikipedia dump holds wikitext, not text. For a search index only the
//! first few hundred characters of prose matter (they say what the article is
//! about), so this does not try to render wikitext. It removes what is not
//! prose (templates, tables, references, files, categories, markup) and keeps
//! the rest, which is good enough to rank pages and to show a snippet.

/// How much of an article is scanned before giving up on finding prose.
/// Infoboxes can run to tens of kilobytes.
const SCAN_CHARS: usize = 60_000;

/// Decode the five XML entities and numeric references of a dump `<text>`.
pub fn decode_xml(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';').filter(|end| *end <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity.strip_prefix('#').and_then(|number| {
                let code = match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => number.parse().ok()?,
                };
                char::from_u32(code)
            }),
        };
        match decoded {
            Some(character) => {
                out.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The opening prose of an article, at most `max_chars` characters, with
/// markup removed and whitespace collapsed. Empty when the page holds no prose.
pub fn lead(wikitext: &str, max_chars: usize) -> String {
    let scanned: String = wikitext.chars().take(SCAN_CHARS).collect();
    let text = strip_nested(&scanned, "<!--", "-->");
    let text = strip_tags_with_content(&text);
    let text = strip_nested(&text, "{{", "}}");
    let text = strip_nested(&text, "{|", "|}");
    let mut kept = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || is_markup_line(line) {
            continue;
        }
        let line = strip_links(line);
        let line = line.replace("'''", "").replace("''", "");
        let line = line.trim_matches(|c: char| c == '=' || c.is_whitespace());
        let line = strip_html_tags(line);
        if line.chars().filter(|c| c.is_alphabetic()).count() < 3 {
            continue;
        }
        if !kept.is_empty() {
            kept.push(' ');
        }
        kept.push_str(line.trim());
        if kept.chars().count() >= max_chars {
            break;
        }
    }
    let collapsed = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(max_chars).collect()
}

/// Lines that are structure, not prose.
fn is_markup_line(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    line.starts_with(['*', '#', ':', ';', '|', '!', '}'])
        || lowered.starts_with("[[file:")
        || lowered.starts_with("[[image:")
        || lowered.starts_with("[[category:")
        || lowered.starts_with("[[media:")
        || lowered.starts_with("__")
}

/// Remove `open ... close` spans, nested ones included. An unclosed span
/// removes the rest of the text.
fn strip_nested(text: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < text.len() {
        let rest = &text[index..];
        if rest.starts_with(open) {
            depth += 1;
            index += open.len();
        } else if depth > 0 && rest.starts_with(close) {
            depth -= 1;
            index += close.len();
        } else {
            let character = rest.chars().next().unwrap_or(' ');
            if depth == 0 {
                out.push(character);
            }
            index += character.len_utf8();
        }
    }
    out
}

/// Remove `<ref>…</ref>`, `<ref … />`, `<gallery>`, `<math>` and similar tags
/// together with what they enclose.
fn strip_tags_with_content(text: &str) -> String {
    const TAGS: &[&str] = &[
        "ref",
        "gallery",
        "math",
        "timeline",
        "imagemap",
        "score",
        "syntaxhighlight",
        "source",
        "code",
        "pre",
        "nowiki",
        "references",
        "noinclude",
        "mapframe",
        "chem",
    ];
    let mut out = text.to_string();
    for tag in TAGS {
        // ASCII lower-casing keeps every byte offset, so one lowered copy
        // serves the whole pass.
        let lowered = out.to_ascii_lowercase();
        let open = format!("<{tag}");
        if !lowered.contains(&open) {
            continue;
        }
        let close = format!("</{tag}>");
        let mut result = String::with_capacity(out.len());
        let mut position = 0usize;
        while let Some(start) = find_tag_start(&lowered[position..], &open).map(|at| position + at)
        {
            result.push_str(&out[position..start]);
            let tag_end = lowered[start..]
                .find('>')
                .map(|end| start + end + 1)
                .unwrap_or(out.len());
            if out[start..tag_end].ends_with("/>") {
                position = tag_end;
                continue;
            }
            match lowered[tag_end..].find(&close) {
                Some(end) => position = tag_end + end + close.len(),
                None => {
                    position = out.len();
                    break;
                }
            }
        }
        result.push_str(&out[position..]);
        out = result;
    }
    out
}

/// Position of `<tag` that is a whole tag name (`<ref>` and `<ref name=…>`,
/// not `<references>`).
fn find_tag_start(lowered: &str, open: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(found) = lowered[from..].find(open) {
        let position = from + found;
        let next = lowered[position + open.len()..].chars().next();
        if matches!(next, None | Some('>' | '/' | ' ' | '\n' | '\t')) {
            return Some(position);
        }
        from = position + open.len();
    }
    None
}

/// `[[target|label]]` becomes `label`, `[[target]]` becomes `target`, and
/// `[http://x label]` becomes `label`.
fn strip_links(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find("[[") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else {
            rest = after;
            break;
        };
        let inner = &after[..end];
        let lowered = inner.to_ascii_lowercase();
        if !(lowered.starts_with("file:")
            || lowered.starts_with("image:")
            || lowered.starts_with("category:"))
        {
            out.push_str(inner.rsplit('|').next().unwrap_or(inner));
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    let mut text = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(start) = rest.find('[') {
        text.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find(']') {
            Some(end) if after.starts_with("http") || after.starts_with("//") => {
                let inner = &after[..end];
                if let Some((_, label)) = inner.split_once(' ') {
                    text.push_str(label);
                }
                rest = &after[end + 1..];
            }
            _ => {
                text.push('[');
                rest = after;
            }
        }
    }
    text.push_str(rest);
    text
}

/// Remove remaining `<…>` tags, keeping their text.
fn strip_html_tags(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_tag = false;
    for character in line.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(character),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_entities_are_decoded_once() {
        assert_eq!(
            decode_xml("a &lt;ref&gt;x&lt;/ref&gt; &amp; b &quot;c&quot; &#169; &#x41;"),
            "a <ref>x</ref> & b \"c\" © A"
        );
        assert_eq!(
            decode_xml("fish & chips; &unknown;"),
            "fish & chips; &unknown;"
        );
    }

    #[test]
    fn the_lead_skips_templates_and_keeps_prose() {
        let text = "{{Infobox person\n| name = Ada\n| nested = {{birth date|1815|12|10}}\n}}\n\
                    '''Ada Lovelace''' (1815–1852) was an [[England|English]] [[mathematician]] \
                    known for her work on the [[Analytical Engine]].<ref name=a>cite</ref>\n\
                    [[File:Ada.jpg|thumb|Portrait]]\n== Early life ==\nShe was born in London.";
        let lead = lead(text, 200);
        assert!(lead.starts_with("Ada Lovelace (1815–1852) was an English mathematician"));
        assert!(lead.contains("Analytical Engine."));
        assert!(!lead.contains("Infobox") && !lead.contains("cite") && !lead.contains("Portrait"));
        assert!(lead.contains("Early life She was born in London."));
    }

    #[test]
    fn tables_comments_lists_and_categories_are_not_prose() {
        let text = "<!-- hidden -->\n{| class=wikitable\n| a || b\n|}\n* a bullet\n\
                    [[Category:People]]\nThe article is about [https://example.org a thing] only.";
        assert_eq!(lead(text, 100), "The article is about a thing only.");
    }

    #[test]
    fn references_with_attributes_and_self_closing_forms_are_removed() {
        let text = "Text one<ref name=\"x\">source</ref> two<ref name=y /> three.\n";
        assert_eq!(lead(text, 100), "Text one two three.");
    }

    #[test]
    fn the_lead_is_cut_to_the_requested_length() {
        let text = "word ".repeat(500);
        assert_eq!(lead(&text, 50).chars().count(), 50);
        assert_eq!(lead("{{only a template}}", 50), "");
    }
}
