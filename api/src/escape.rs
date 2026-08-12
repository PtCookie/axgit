//! Tiny hand-rolled escaper shared by axgit's two hand-built markup outputs:
//! the Atom feed (`handlers/feed.rs`) and the per-repo `<link>`s injected
//! into the static page shell (`shell.rs`). Both are small, fixed-structure
//! documents built by string concatenation rather than through a templating
//! or XML crate (docs/DECISIONS.md #12), so both need the same escaping —
//! XML's five predefined entities are also sufficient for an HTML5
//! double-quoted attribute value.

/// Escapes `&`, `<`, `>`, `"`, and `'` — enough for both an XML text/attribute
/// node and an HTML5 double-quoted attribute value.
pub(crate) fn xml_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escape_should_escape_all_five_specials() {
        assert_eq!(
            xml_escape(r#"<b> & "it's""#),
            "&lt;b&gt; &amp; &quot;it&apos;s&quot;"
        );
        assert_eq!(xml_escape("plain"), "plain");
    }
}
