//! roxmltree helpers that match elements by canonical namespace prefix, so Transitional and
//! Strict OOXML (and odd prefixes) read the same.

use roxmltree::{Node, ParsingOptions};

pub const OPTS: ParsingOptions = ParsingOptions { allow_dtd: true, nodes_limit: u32::MAX };

/// Canonical prefix of a namespace URI (`"w"`, `"a"`, …); `"?"` for an unknown one, `""` for none.
pub fn prefix(uri: Option<&str>) -> &'static str {
    let Some(uri) = uri else { return "" };
    match uri {
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
        | "http://purl.oclc.org/ooxml/wordprocessingml/main" => "w",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
        | "http://purl.oclc.org/ooxml/officeDocument/relationships" => "r",
        "http://schemas.openxmlformats.org/drawingml/2006/main" | "http://purl.oclc.org/ooxml/drawingml/main" => "a",
        "http://schemas.openxmlformats.org/drawingml/2006/picture"
        | "http://purl.oclc.org/ooxml/drawingml/picture" => "pic",
        "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
        | "http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing" => "wp",
        "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"
        | "http://purl.oclc.org/ooxml/drawingml/spreadsheetDrawing" => "xdr",
        "http://schemas.openxmlformats.org/drawingml/2006/chart" | "http://purl.oclc.org/ooxml/drawingml/chart" => "c",
        "http://schemas.openxmlformats.org/officeDocument/2006/math"
        | "http://purl.oclc.org/ooxml/officeDocument/math" => "m",
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
        | "http://purl.oclc.org/ooxml/spreadsheetml/main" => "x",
        "http://schemas.openxmlformats.org/markup-compatibility/2006" => "mc",
        "urn:schemas-microsoft-com:vml" => "v",
        "urn:schemas-microsoft-com:office:office" => "o",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingShape" => "wps",
        "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup" => "wpg",
        "http://schemas.microsoft.com/office/word/2010/wordml" => "w14",
        "http://www.w3.org/XML/1998/namespace" => "xml",
        "http://schemas.openxmlformats.org/package/2006/relationships" => "pr",
        _ => "?",
    }
}

/// Whether `n` is the element `pfx:name`.
#[inline]
pub fn is(n: Node, pfx: &str, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name && prefix(n.tag_name().namespace()) == pfx
}

/// `pfx:name` of an element (`"w:p"`), for matching in `match` arms.
pub fn qname(n: Node) -> String {
    let p = prefix(n.tag_name().namespace());
    if p.is_empty() {
        n.tag_name().name().to_string()
    } else {
        format!("{p}:{}", n.tag_name().name())
    }
}

/// First child element `pfx:name`.
pub fn child<'a, 'i>(n: Node<'a, 'i>, pfx: &str, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| is(*c, pfx, name))
}

/// Every child element `pfx:name`.
pub fn children<'a, 'i: 'a>(n: Node<'a, 'i>, pfx: &'a str, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    n.children().filter(move |c| is(*c, pfx, name))
}

/// Every child element.
pub fn elems<'a, 'i: 'a>(n: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    n.children().filter(|c| c.is_element())
}

/// Attribute `pfx:name` (`pfx` `""` for an unqualified attribute).
pub fn attr<'a>(n: Node<'a, '_>, pfx: &str, name: &str) -> Option<&'a str> {
    n.attributes()
        .find(|a| a.name() == name && prefix(a.namespace()) == pfx)
        .map(|a| a.value())
}

/// `w:val` (WordprocessingML's usual value attribute).
pub fn wval<'a>(n: Node<'a, '_>) -> Option<&'a str> {
    attr(n, "w", "val")
}

/// `w:val` of child `w:name`.
pub fn child_wval<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    child(n, "w", name).and_then(wval)
}

/// A WordprocessingML on/off property: present without `w:val`, or with a true value.
pub fn on_off(v: Option<&str>) -> bool {
    !matches!(v, Some("0") | Some("false") | Some("off") | Some("none"))
}

/// Concatenated text of every descendant text node.
pub fn text(n: Node) -> String {
    let mut s = String::new();
    for d in n.descendants() {
        if d.is_text() {
            s.push_str(d.text().unwrap_or(""));
        }
    }
    s
}

pub fn num<T: std::str::FromStr>(v: Option<&str>) -> Option<T> {
    v.and_then(|s| s.trim().parse().ok())
}

/// A measure in twentieths of a point that may carry a unit suffix (`"720"`, `"1in"`, `"12pt"`, `"2.5cm"`).
pub fn twips(v: Option<&str>) -> Option<f64> {
    let s = v?.trim();
    let (n, unit) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic() || c == '%').unwrap_or(s.len()));
    let n: f64 = n.parse().ok()?;
    Some(match unit {
        "" => n,
        "pt" => n * 20.0,
        "in" => n * 1440.0,
        "cm" => n * 1440.0 / 2.54,
        "mm" => n * 144.0 / 2.54,
        "pc" | "pi" => n * 240.0,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_strict_and_transitional() {
        let t = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body/></w:document>"#;
        let s = r#"<q:document xmlns:q="http://purl.oclc.org/ooxml/wordprocessingml/main"><q:body/></q:document>"#;
        for src in [t, s] {
            let d = roxmltree::Document::parse(src).unwrap();
            assert!(is(d.root_element(), "w", "document"));
            assert!(child(d.root_element(), "w", "body").is_some());
        }
    }

    #[test]
    fn twips_units() {
        assert_eq!(twips(Some("720")), Some(720.0));
        assert_eq!(twips(Some("1in")), Some(1440.0));
        assert_eq!(twips(Some("12pt")), Some(240.0));
    }
}
