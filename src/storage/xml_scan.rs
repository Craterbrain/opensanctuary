//! Minimal hand-rolled XML scanning shared by the format importers/exporters
//! (`pptx_import.rs`, `odp_import.rs`, `odp_export.rs`). Deliberately not a real XML
//! parser/dependency: every format here is read with a narrow, documented set of
//! assumptions (no same-named tag nesting within the scope each helper is handed,
//! no CDATA, no processing instructions) that a full OOXML/ODF document doesn't
//! always satisfy in general, but every shape these importers actually walk does.
//! Pulled out once both pptx_import.rs and odp_import.rs needed exactly the same
//! primitives, rather than duplicating them.

/// Finds the next occurrence of a tag named `tag` (matching both `<tag>`/`<tag attr="...">`
/// and their self-closing `.../>` forms), returning its raw attribute substring, its inner
/// content (empty for a self-closing tag), and the remainder of `xml` just past the tag so
/// the caller can keep scanning forward.
pub fn next_tag<'a>(xml: &'a str, tag: &str) -> Option<(&'a str, &'a str, &'a str)> {
    let open_needle = format!("<{}", tag);
    let mut search_from = 0;
    loop {
        let rel_pos = xml[search_from..].find(&open_needle)?;
        let abs_pos = search_from + rel_pos;
        let after_tag_name = abs_pos + open_needle.len();
        let next_char = xml[after_tag_name..].chars().next();
        // Reject partial matches, e.g. tag "a:t" must not match "a:txBody" or "a:tc".
        if !matches!(next_char, Some('>') | Some(' ') | Some('/')) {
            search_from = after_tag_name;
            continue;
        }
        let gt_pos = abs_pos + xml[abs_pos..].find('>')?;
        let attrs = &xml[after_tag_name..gt_pos];
        if xml[..gt_pos].ends_with('/') {
            let attrs = attrs.strip_suffix('/').unwrap_or(attrs);
            return Some((attrs, "", &xml[gt_pos + 1..]));
        }
        let close_needle = format!("</{}>", tag);
        let content_start = gt_pos + 1;
        let close_pos = content_start + xml[content_start..].find(&close_needle)?;
        return Some((attrs, &xml[content_start..close_pos], &xml[close_pos + close_needle.len()..]));
    }
}

/// Content-only convenience wrapper around `next_tag`, for tags whose attributes aren't
/// needed (e.g. `<a:t>`, `<text:p>`).
pub fn next_tag_content<'a>(xml: &'a str, tag: &str) -> Option<(&'a str, &'a str)> {
    let (_, content, remainder) = next_tag(xml, tag)?;
    Some((content, remainder))
}

/// Iterates all occurrences of `<tag_name ...>` / `<tag_name .../>`, yielding the raw
/// attribute substring between the tag name and its closing `>`/`/>` for each.
pub fn find_all_tag_attrs<'a>(xml: &'a str, tag_name: &str) -> Vec<&'a str> {
    let mut results = Vec::new();
    let open_needle = format!("<{}", tag_name);
    let mut search_from = 0;
    while let Some(rel_pos) = xml[search_from..].find(&open_needle) {
        let abs_pos = search_from + rel_pos;
        let after_tag_name = abs_pos + open_needle.len();
        let next_char = xml[after_tag_name..].chars().next();
        if !matches!(next_char, Some('>') | Some(' ') | Some('/')) {
            search_from = after_tag_name;
            continue;
        }
        let Some(gt_rel) = xml[abs_pos..].find('>') else { break };
        let gt_pos = abs_pos + gt_rel;
        results.push(&xml[after_tag_name..gt_pos]);
        search_from = gt_pos + 1;
    }
    results
}

/// Position of the next `<tag` occurrence that's actually a tag-name match (not a
/// same-prefixed longer name, e.g. `p:sp` must not match inside `p:spPr`) -- the same
/// partial-match guard `next_tag` itself uses, exposed separately for callers (like
/// `next_slide_shape`/`next_odp_shape`) that need to compare *where* two different tag
/// names would each be found without consuming either match.
pub fn find_tag_open_pos(xml: &str, tag: &str) -> Option<usize> {
    let needle = format!("<{}", tag);
    let mut search_from = 0;
    loop {
        let rel = xml[search_from..].find(&needle)?;
        let abs = search_from + rel;
        let after = abs + needle.len();
        match xml[after..].chars().next() {
            Some('>') | Some(' ') | Some('/') => return Some(abs),
            _ => search_from = after,
        }
    }
}

/// Like `next_tag`, but correct when `tag` nests inside itself (e.g. ODP's `draw:g`, which can
/// contain another `draw:g`) -- `next_tag` finds the *first* closing tag, which for nested same-
/// named tags is the wrong one (the inner group's close, not the outer one's). This instead walks
/// forward counting nested opens/closes (skipping self-closing `<tag/>` opens, which never need a
/// matching close), so it returns the whole outer span even when it contains further nesting.
pub fn extract_balanced_tag<'a>(xml: &'a str, tag: &str) -> Option<(&'a str, &'a str, &'a str)> {
    let open_pos = find_tag_open_pos(xml, tag)?;
    let open_needle = format!("<{}", tag);
    let after_tag_name = open_pos + open_needle.len();
    let first_gt = open_pos + xml[open_pos..].find('>')?;
    let attrs = &xml[after_tag_name..first_gt];
    if xml[..first_gt].ends_with('/') {
        let attrs = attrs.strip_suffix('/').unwrap_or(attrs);
        return Some((attrs, "", &xml[first_gt + 1..]));
    }
    let close_needle = format!("</{}>", tag);
    let mut depth = 1i32;
    let mut pos = first_gt + 1;
    loop {
        let next_open_abs = find_tag_open_pos(&xml[pos..], tag).map(|p| pos + p);
        let next_close_abs = xml[pos..].find(&close_needle).map(|p| pos + p);
        match (next_open_abs, next_close_abs) {
            (Some(o), Some(c)) if o < c => {
                let gt = o + xml[o..].find('>')?;
                if !xml[..gt].ends_with('/') {
                    depth += 1;
                }
                pos = gt + 1;
            }
            (_, Some(c)) => {
                depth -= 1;
                if depth == 0 {
                    return Some((attrs, &xml[first_gt + 1..c], &xml[c + close_needle.len()..]));
                }
                pos = c + close_needle.len();
            }
            _ => return None, // unbalanced/malformed -- bail rather than guess
        }
    }
}

pub fn extract_attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{}=\"", name);
    let start = attrs.find(&needle)? + needle.len();
    let end = start + attrs[start..].find('"')?;
    Some(&attrs[start..end])
}

/// Decodes the 5 standard XML entities. `&amp;` must be decoded last, otherwise a
/// literal `&amp;lt;` in the source would incorrectly decode to `<` instead of `&lt;`.
pub fn decode_xml_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The inverse of `decode_xml_entities`, for writers: escapes the characters that are never
/// literal in XML text content or a quoted attribute value. `&` first, so it doesn't double-escape
/// the ones it just introduced.
pub fn encode_xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_tag_handles_self_closing_and_normal_forms_and_partial_name_guard() {
        assert_eq!(next_tag("<a:t>hi</a:t> rest", "a:t"), Some(("", "hi", " rest")));
        assert_eq!(next_tag("<a:t/> rest", "a:t"), Some(("", "", " rest")));
        assert_eq!(next_tag("<a:t attr=\"1\">hi</a:t>", "a:t").map(|(a, c, _)| (a, c)), Some((" attr=\"1\"", "hi")));
        // "a:t" must not match inside "a:txBody" or "a:tc".
        assert_eq!(next_tag("<a:txBody><a:t>real</a:t></a:txBody>", "a:t"), Some(("", "real", "</a:txBody>")));
        assert_eq!(next_tag("no tag here", "a:t"), None);
    }

    #[test]
    fn find_all_tag_attrs_and_extract_attr_roundtrip() {
        let xml = r#"<p:sp foo="1"/><p:sp foo="2" bar="x"/>"#;
        let attrs = find_all_tag_attrs(xml, "p:sp");
        assert_eq!(attrs.len(), 2);
        assert_eq!(extract_attr(attrs[0], "foo"), Some("1"));
        assert_eq!(extract_attr(attrs[1], "bar"), Some("x"));
        assert_eq!(extract_attr(attrs[1], "missing"), None);
    }

    #[test]
    fn extract_balanced_tag_handles_nesting_self_closing_and_siblings() {
        // A nested same-named tag: next_tag would wrongly stop at the inner </draw:g>.
        let xml = r#"<draw:g a="1"><draw:rect/><draw:g a="2"><draw:rect/></draw:g><draw:rect/></draw:g><draw:rect id="after"/>"#;
        let (attrs, inner, remainder) = extract_balanced_tag(xml, "draw:g").unwrap();
        assert_eq!(attrs, r#" a="1""#);
        assert_eq!(inner, r#"<draw:rect/><draw:g a="2"><draw:rect/></draw:g><draw:rect/>"#);
        assert_eq!(remainder, r#"<draw:rect id="after"/>"#);

        // Self-closing: no content, remainder starts right after.
        assert_eq!(extract_balanced_tag(r#"<draw:g/> rest"#, "draw:g"), Some(("", "", " rest")));

        // A self-closing same-named tag inside must not be treated as an unmatched open.
        let xml2 = r#"<draw:g><draw:g/><draw:rect/></draw:g>tail"#;
        let (_, inner2, remainder2) = extract_balanced_tag(xml2, "draw:g").unwrap();
        assert_eq!(inner2, r#"<draw:g/><draw:rect/>"#);
        assert_eq!(remainder2, "tail");

        assert_eq!(extract_balanced_tag("no tag here", "draw:g"), None);
    }

    #[test]
    fn entity_decode_and_encode_round_trip_and_amp_ordering() {
        assert_eq!(decode_xml_entities("&amp;lt;"), "&lt;"); // &amp; decoded last
        assert_eq!(decode_xml_entities("&lt;b&gt; &quot;x&quot; &apos;y&apos; &amp;"), "<b> \"x\" 'y' &");
        let raw = "<b> \"x\" 'y' & ok";
        assert_eq!(decode_xml_entities(&encode_xml_text(raw)), raw);
    }
}
