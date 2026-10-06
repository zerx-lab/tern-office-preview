//! `xl/sharedStrings.xml` and inline strings: plain text plus rich-text runs.

use super::styles::{ColorCtx, Font};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashMap;

/// A styled piece of a rich string.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub class: String,
}

/// One `<si>` / `<is>`: its text, and runs when any of them carries formatting.
#[derive(Default, Debug)]
pub struct Si {
    pub text: String,
    pub runs: Vec<Run>,
}

/// The shared string table, compact: all text in one arena, rich runs on the side.
#[derive(Default)]
pub struct Strings {
    text: String,
    spans: Vec<(u32, u32)>,
    rich: HashMap<u32, Vec<Run>>,
}

impl Strings {
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub fn get(&self, i: u32) -> Option<&str> {
        let (s, l) = *self.spans.get(i as usize)?;
        self.text.get(s as usize..(s + l) as usize)
    }

    pub fn rich(&self, i: u32) -> Option<&[Run]> {
        self.rich.get(&i).map(|v| v.as_slice())
    }

    pub fn parse(bytes: &[u8], ctx: &ColorCtx) -> Strings {
        let mut st = Strings::default();
        let mut r = Reader::from_reader(bytes);
        loop {
            match r.read_event() {
                Ok(Event::Start(e)) if e.local_name().as_ref() == b"si" => {
                    let si = read_si(&mut r, ctx, b"si");
                    let start = st.text.len() as u32;
                    st.text.push_str(&si.text);
                    let idx = st.spans.len() as u32;
                    st.spans.push((start, si.text.len() as u32));
                    if !si.runs.is_empty() {
                        st.rich.insert(idx, si.runs);
                    }
                }
                Ok(Event::Empty(e)) if e.local_name().as_ref() == b"si" => {
                    st.spans.push((st.text.len() as u32, 0));
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        st
    }
}

/// Attributes of an element by local name.
pub fn attrs_of(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes()
        .with_checks(false)
        .flatten()
        .map(|a| {
            (
                String::from_utf8_lossy(a.key.local_name().as_ref()).into_owned(),
                a.unescape_value().map(|v| v.into_owned()).unwrap_or_default(),
            )
        })
        .collect()
}

fn apply_font(font: &mut Font, e: &BytesStart, ctx: &ColorCtx) {
    let attrs = attrs_of(e);
    let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
    font.apply(&name, &|k| attrs.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone()), ctx);
}

/// Reads the children of a `<si>` / `<is>` up to its end tag (`end` is the local name).
pub fn read_si(r: &mut Reader<&[u8]>, ctx: &ColorCtx, end: &[u8]) -> Si {
    let mut si = Si::default();
    let mut in_t = false;
    let mut in_run = false;
    let mut in_rpr = false;
    let mut has_rpr = false;
    let mut font = Font::default();
    let mut run = String::new();
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"t" => in_t = true,
                b"r" => {
                    in_run = true;
                    has_rpr = false;
                    run.clear();
                }
                b"rPr" => {
                    in_rpr = true;
                    has_rpr = true;
                    font = Font::default();
                }
                b"rPh" => {
                    let _ = r.read_to_end(e.name());
                }
                _ if in_rpr => apply_font(&mut font, &e, ctx),
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if in_rpr {
                    apply_font(&mut font, &e, ctx);
                }
            }
            Ok(Event::Text(t)) if in_t => {
                let s =
                    t.unescape().map(|c| c.into_owned()).unwrap_or_else(|_| String::from_utf8_lossy(&t).into_owned());
                si.text.push_str(&s);
                if in_run {
                    run.push_str(&s);
                }
            }
            Ok(Event::CData(t)) if in_t => {
                let s = String::from_utf8_lossy(&t);
                si.text.push_str(&s);
                if in_run {
                    run.push_str(&s);
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"t" => in_t = false,
                b"rPr" => in_rpr = false,
                b"r" => {
                    in_run = false;
                    si.runs.push(Run {
                        text: unescape_x(std::mem::take(&mut run)),
                        class: if has_rpr { font.run_class() } else { String::new() },
                    });
                }
                n if n == end => break,
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    si.text = unescape_x(si.text);
    if si.runs.iter().all(|r| r.class.is_empty()) {
        si.runs.clear();
    } else {
        si.runs.retain(|r| !r.text.is_empty());
    }
    si
}

/// Decodes Excel's `_xHHHH_` escapes for control characters (`_x000D_` → CR).
pub fn unescape_x(s: String) -> String {
    if !s.contains("_x") {
        return s;
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut last = 0;
    while i < b.len() {
        if b[i] == b'_'
            && i + 7 <= b.len()
            && b[i + 1] == b'x'
            && b[i + 6] == b'_'
            && b[i + 2..i + 6].iter().all(|c| c.is_ascii_hexdigit())
        {
            if let Some(c) = u32::from_str_radix(&s[i + 2..i + 6], 16).ok().and_then(char::from_u32) {
                out.push_str(&s[last..i]);
                out.push(c);
                i += 7;
                last = i;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&s[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Theme;

    fn table(xml: &str) -> Strings {
        let theme = Theme::default();
        let ctx = ColorCtx { theme: &theme, indexed: &[] };
        Strings::parse(xml.as_bytes(), &ctx)
    }

    #[test]
    fn plain_and_rich() {
        let s = table(
            r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="3">
<si><t>hello</t></si>
<si><r><rPr><b/><sz val="14"/><color rgb="FFFF0000"/><rFont val="Arial"/></rPr><t>Big</t></r><r><t xml:space="preserve"> plain &amp; more</t></r></si>
<si><t>a</t><rPh sb="0" eb="1"><t>ignored</t></rPh><phoneticPr fontId="1"/></si>
<si/>
</sst>"#,
        );
        assert_eq!(s.len(), 4);
        assert_eq!(s.get(0), Some("hello"));
        assert!(s.rich(0).is_none());
        assert_eq!(s.get(1), Some("Big plain & more"));
        let runs = s.rich(1).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0], Run { text: "Big".into(), class: "b f28 ffh c700".into() });
        assert_eq!(runs[1], Run { text: " plain & more".into(), class: String::new() });
        assert_eq!(s.get(2), Some("a"));
        assert_eq!(s.get(3), Some(""));
        assert_eq!(s.get(4), None);
    }

    #[test]
    fn run_fonts() {
        let s = table(
            r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<si><r><rPr><i/><u/><strike/><vertAlign val="superscript"/></rPr><t>x</t></r><r><rPr><u val="double"/></rPr><t>y</t></r></si></sst>"#,
        );
        let runs = s.rich(0).unwrap();
        assert_eq!(runs[0].class, "nb i us sup");
        assert_eq!(runs[1].class, "nb uu");
    }

    #[test]
    fn escapes() {
        assert_eq!(unescape_x("a_x000D__x000A_b".into()), "a\r\nb");
        assert_eq!(unescape_x("_x4E2D_文".into()), "中文");
        assert_eq!(unescape_x("snake_x_case".into()), "snake_x_case");
        assert_eq!(unescape_x("plain".into()), "plain");
    }
}
