//! Run / paragraph / border / shading properties as the file states them, plus the layering
//! (`overlay`) the style resolution is built from.

use crate::color::{self, Rgb, Theme};
use crate::css::{Border, Line};
use crate::xml::{attr, child, elems, num, on_off, twips, wval};
use roxmltree::Node;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UKind {
    None,
    Single,
    Double,
    Dotted,
    Dash,
    Wave,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Vert {
    Base,
    Sup,
    Sub,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Jc {
    Left,
    Center,
    Right,
    Both,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineRule {
    Auto,
    Exact,
    AtLeast,
}

/// Character properties; `None` = not stated (inherits). Colors/fills are `Some(None)` when the
/// file says "auto" / "none" explicitly.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct RPr {
    pub b: Option<bool>,
    pub i: Option<bool>,
    pub strike: Option<bool>,
    pub dstrike: Option<bool>,
    pub caps: Option<bool>,
    pub small_caps: Option<bool>,
    pub vanish: Option<bool>,
    pub u: Option<UKind>,
    pub color: Option<Option<Rgb>>,
    /// Half-points.
    pub size: Option<f64>,
    pub highlight: Option<Option<Rgb>>,
    pub shd: Option<Option<Rgb>>,
    pub vert: Option<Vert>,
    pub font_ascii: Option<Rc<str>>,
    pub font_ea: Option<Rc<str>>,
    /// `w:rStyle` (only meaningful on direct formatting; `overlay` ignores it).
    pub rstyle: Option<Rc<str>>,
}

macro_rules! over {
    ($s:ident, $o:ident, $($f:ident),*) => {
        $( if $o.$f.is_some() { $s.$f = $o.$f.clone(); } )*
    };
}

impl RPr {
    pub fn overlay(&mut self, o: &RPr) {
        over!(
            self, o, b, i, strike, dstrike, caps, small_caps, vanish, u, color, size, highlight, shd, vert,
            font_ascii, font_ea
        );
    }

    pub fn parse(n: Node, theme: &Theme) -> RPr {
        let mut r = RPr::default();
        for c in elems(n) {
            if crate::xml::prefix(c.tag_name().namespace()) != "w" {
                continue;
            }
            let v = wval(c);
            match c.tag_name().name() {
                "b" => r.b = Some(on_off(v)),
                "i" => r.i = Some(on_off(v)),
                "strike" => r.strike = Some(on_off(v)),
                "dstrike" => r.dstrike = Some(on_off(v)),
                "caps" => r.caps = Some(on_off(v)),
                "smallCaps" => r.small_caps = Some(on_off(v)),
                "vanish" | "webHidden" => {
                    if on_off(v) {
                        r.vanish = Some(true)
                    } else if c.tag_name().name() == "vanish" {
                        r.vanish = Some(false)
                    }
                }
                "u" => {
                    r.u = Some(match v.unwrap_or("single") {
                        "none" => UKind::None,
                        "double" => UKind::Double,
                        "dotted" | "dottedHeavy" => UKind::Dotted,
                        "dash" | "dashedHeavy" | "dashLong" | "dashLongHeavy" | "dotDash" | "dashDotHeavy"
                        | "dotDotDash" | "dashDotDotHeavy" => UKind::Dash,
                        "wave" | "wavyHeavy" | "wavyDouble" => UKind::Wave,
                        _ => UKind::Single,
                    })
                }
                "color" => {
                    if let Some(c) = themed(c, theme, "val", "themeColor", "themeTint", "themeShade") {
                        r.color = Some(c);
                    }
                }
                "sz" => {
                    if let Some(s) = num::<f64>(v) {
                        r.size = Some(s);
                    }
                }
                "highlight" => {
                    r.highlight = Some(match v {
                        Some("none") | None => None,
                        Some(name) => color::highlight(name),
                    })
                }
                "shd" => {
                    if let Some(f) = shd_fill(c, theme) {
                        r.shd = Some(f);
                    }
                }
                "vertAlign" => {
                    r.vert = Some(match v {
                        Some("superscript") => Vert::Sup,
                        Some("subscript") => Vert::Sub,
                        _ => Vert::Base,
                    })
                }
                "rFonts" => {
                    let theme_font = |name: &str| -> Option<Rc<str>> {
                        let f = match name {
                            "minorHAnsi" | "minorAscii" | "minorBidi" => &theme.minor_latin,
                            "majorHAnsi" | "majorAscii" | "majorBidi" => &theme.major_latin,
                            "minorEastAsia" => &theme.minor_ea,
                            "majorEastAsia" => &theme.major_ea,
                            _ => return None,
                        };
                        (!f.is_empty()).then(|| Rc::from(f.as_str()))
                    };
                    let a = attr(c, "w", "ascii")
                        .or_else(|| attr(c, "w", "hAnsi"))
                        .map(Rc::from)
                        .or_else(|| attr(c, "w", "asciiTheme").and_then(theme_font))
                        .or_else(|| attr(c, "w", "hAnsiTheme").and_then(theme_font));
                    let e = attr(c, "w", "eastAsia")
                        .map(Rc::from)
                        .or_else(|| attr(c, "w", "eastAsiaTheme").and_then(theme_font));
                    if a.is_some() {
                        r.font_ascii = a;
                    }
                    if e.is_some() {
                        r.font_ea = e;
                    }
                }
                "rStyle" => r.rstyle = v.map(Rc::from),
                _ => {}
            }
        }
        r
    }
}

/// `w:color`-like element: `val` hex, else a theme color (with tint/shade), else `auto`.
/// `Some(None)` is an explicit "auto".
pub fn themed(
    n: Node,
    theme: &Theme,
    val: &str,
    theme_attr: &str,
    tint: &str,
    shade: &str,
) -> Option<Option<Rgb>> {
    let hex = attr(n, "w", val);
    if let Some(c) = hex.and_then(Rgb::hex) {
        return Some(Some(c));
    }
    if let Some(mut c) = attr(n, "w", theme_attr).and_then(|t| theme.scheme(t)) {
        let f = |s: &str| u8::from_str_radix(attr(n, "w", s)?, 16).ok().map(|v| v as f64 / 255.0);
        if let Some(t) = f(tint) {
            c = Rgb::WHITE.mix(c, t);
        }
        if let Some(s) = f(shade) {
            c = Rgb::BLACK.mix(c, s);
        }
        return Some(Some(c));
    }
    if hex == Some("auto") {
        return Some(None);
    }
    None
}

/// The background a `w:shd` paints: `fill`, `solid` → color, `pctNN` → fill/color blend.
/// `Some(None)` = explicitly no fill.
pub fn shd_fill(n: Node, theme: &Theme) -> Option<Option<Rgb>> {
    let fill = themed(n, theme, "fill", "themeFill", "themeFillTint", "themeFillShade");
    let pat = themed(n, theme, "color", "themeColor", "themeTint", "themeShade");
    let val = wval(n).unwrap_or("clear");
    if val == "nil" || val == "none" {
        return Some(None);
    }
    let fill_c = fill.flatten();
    let pat_c = pat.flatten().unwrap_or(Rgb::BLACK);
    if val == "solid" {
        return Some(Some(pat_c));
    }
    if let Some(p) = val.strip_prefix("pct").and_then(|p| p.parse::<f64>().ok()) {
        return Some(Some(fill_c.unwrap_or(Rgb::WHITE).mix(pat_c, p / 100.0)));
    }
    if val.starts_with("horz") || val.starts_with("vert") || val.starts_with("diag") {
        return Some(Some(fill_c.unwrap_or(Rgb::WHITE).mix(pat_c, 0.25)));
    }
    fill
}

/// One border: `line` is `None` for "no border".
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Bdr {
    pub line: Option<Line>,
    pub color: Option<Rgb>,
}

impl Bdr {
    pub fn parse(n: Node, theme: &Theme) -> Bdr {
        let val = wval(n).unwrap_or("single");
        let sz: f64 = num(attr(n, "w", "sz")).unwrap_or(4.0);
        let color = themed(n, theme, "color", "themeColor", "themeTint", "themeShade").flatten();
        let line = match val {
            "nil" | "none" => None,
            "double" | "triple" | "thinThickSmallGap" | "thickThinSmallGap" | "thinThickThinSmallGap"
            | "thinThickMediumGap" | "thickThinMediumGap" | "thinThickThinMediumGap" | "thinThickLargeGap"
            | "thickThinLargeGap" | "thinThickThinLargeGap" => Some(Line::Double),
            "dashed" | "dashSmallGap" | "dotDash" | "dotDotDash" => Some(Line::Dashed),
            "dotted" => Some(Line::Dotted),
            _ => Some(if val == "thick" || sz >= 18.0 {
                Line::Thick
            } else if sz >= 11.0 {
                Line::Medium
            } else {
                Line::Thin
            }),
        };
        Bdr { line, color }
    }

    pub fn border(&self) -> Option<Border> {
        self.line.map(|line| Border { line, color: self.color })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BKey {
    Top,
    Left,
    Bottom,
    Right,
    InsideH,
    InsideV,
}

/// A set of borders (paragraph, cell or table), each possibly unstated.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Bdrs {
    pub top: Option<Bdr>,
    pub left: Option<Bdr>,
    pub bottom: Option<Bdr>,
    pub right: Option<Bdr>,
    pub inside_h: Option<Bdr>,
    pub inside_v: Option<Bdr>,
}

impl Bdrs {
    pub fn parse(n: Node, theme: &Theme) -> Bdrs {
        let mut b = Bdrs::default();
        for c in elems(n) {
            let v = Some(Bdr::parse(c, theme));
            match c.tag_name().name() {
                "top" => b.top = v,
                "left" | "start" => b.left = v,
                "bottom" => b.bottom = v,
                "right" | "end" => b.right = v,
                "insideH" => b.inside_h = v,
                "insideV" => b.inside_v = v,
                _ => {}
            }
        }
        b
    }

    pub fn get(&self, k: BKey) -> Option<Bdr> {
        match k {
            BKey::Top => self.top,
            BKey::Left => self.left,
            BKey::Bottom => self.bottom,
            BKey::Right => self.right,
            BKey::InsideH => self.inside_h,
            BKey::InsideV => self.inside_v,
        }
    }

    pub fn overlay(&mut self, o: &Bdrs) {
        over!(self, o, top, left, bottom, right, inside_h, inside_v);
    }

}

/// Paragraph properties. Indents / spacing are twips.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct PPr {
    pub jc: Option<Jc>,
    pub left: Option<f64>,
    pub right: Option<f64>,
    /// First-line indent; negative for a hanging indent.
    pub first: Option<f64>,
    /// Indents given in hundredths of a character.
    pub left_chars: Option<f64>,
    pub first_chars: Option<f64>,
    pub before: Option<f64>,
    pub after: Option<f64>,
    pub before_lines: Option<f64>,
    pub after_lines: Option<f64>,
    pub before_auto: Option<bool>,
    pub after_auto: Option<bool>,
    pub line: Option<f64>,
    pub line_rule: Option<LineRule>,
    pub contextual: Option<bool>,
    pub shd: Option<Option<Rgb>>,
    pub borders: Bdrs,
    pub outline: Option<u8>,
    pub num_id: Option<u32>,
    pub ilvl: Option<u8>,
    pub page_break_before: Option<bool>,
    /// `w:pStyle` (direct formatting only).
    pub style: Option<Rc<str>>,
}

impl PPr {
    pub fn overlay(&mut self, o: &PPr) {
        over!(
            self,
            o,
            jc,
            left,
            right,
            first,
            left_chars,
            first_chars,
            before,
            after,
            before_lines,
            after_lines,
            before_auto,
            after_auto,
            line,
            line_rule,
            contextual,
            shd,
            outline,
            num_id,
            ilvl,
            page_break_before
        );
        self.borders.overlay(&o.borders);
    }

    pub fn parse(n: Node, theme: &Theme) -> PPr {
        let mut p = PPr::default();
        for c in elems(n) {
            if crate::xml::prefix(c.tag_name().namespace()) != "w" {
                continue;
            }
            let v = wval(c);
            match c.tag_name().name() {
                "jc" => {
                    p.jc = Some(match v {
                        Some("center") => Jc::Center,
                        Some("right") | Some("end") => Jc::Right,
                        Some("both") | Some("distribute") | Some("thaiDistribute") | Some("mediumKashida") => {
                            Jc::Both
                        }
                        _ => Jc::Left,
                    })
                }
                "ind" => {
                    let t = |names: &[&str]| names.iter().find_map(|n| twips(attr(c, "w", n)));
                    if let Some(l) = t(&["left", "start"]) {
                        p.left = Some(l);
                    }
                    if let Some(r) = t(&["right", "end"]) {
                        p.right = Some(r);
                    }
                    if let Some(h) = t(&["hanging"]) {
                        p.first = Some(-h);
                    } else if let Some(f) = t(&["firstLine"]) {
                        p.first = Some(f);
                    }
                    if let Some(l) = num::<f64>(attr(c, "w", "leftChars").or_else(|| attr(c, "w", "startChars"))) {
                        p.left_chars = Some(l);
                    }
                    if let Some(h) = num::<f64>(attr(c, "w", "hangingChars")) {
                        p.first_chars = Some(-h);
                    } else if let Some(f) = num::<f64>(attr(c, "w", "firstLineChars")) {
                        p.first_chars = Some(f);
                    }
                }
                "spacing" => {
                    if let Some(b) = twips(attr(c, "w", "before")) {
                        p.before = Some(b);
                    }
                    if let Some(a) = twips(attr(c, "w", "after")) {
                        p.after = Some(a);
                    }
                    if let Some(b) = num::<f64>(attr(c, "w", "beforeLines")) {
                        p.before_lines = Some(b);
                    }
                    if let Some(a) = num::<f64>(attr(c, "w", "afterLines")) {
                        p.after_lines = Some(a);
                    }
                    if let Some(b) = attr(c, "w", "beforeAutospacing") {
                        p.before_auto = Some(on_off(Some(b)));
                    }
                    if let Some(a) = attr(c, "w", "afterAutospacing") {
                        p.after_auto = Some(on_off(Some(a)));
                    }
                    if let Some(l) = num::<f64>(attr(c, "w", "line")) {
                        p.line = Some(l);
                        p.line_rule = Some(match attr(c, "w", "lineRule") {
                            Some("exact") => LineRule::Exact,
                            Some("atLeast") => LineRule::AtLeast,
                            _ => LineRule::Auto,
                        });
                    }
                }
                "contextualSpacing" => p.contextual = Some(on_off(v)),
                "shd" => {
                    if let Some(f) = shd_fill(c, theme) {
                        p.shd = Some(f);
                    }
                }
                "pBdr" => p.borders = Bdrs::parse(c, theme),
                "outlineLvl" => p.outline = num::<u8>(v),
                "numPr" => {
                    if let Some(id) = child(c, "w", "numId").and_then(wval) {
                        p.num_id = num::<u32>(Some(id));
                    }
                    if let Some(l) = child(c, "w", "ilvl").and_then(wval) {
                        p.ilvl = num::<u8>(Some(l));
                    }
                }
                "pageBreakBefore" => p.page_break_before = Some(on_off(v)),
                "pStyle" => p.style = v.map(Rc::from),
                _ => {}
            }
        }
        p
    }
}

/// Whether `c` is in the CJK / full-width ranges that want the East Asian font.
pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F
        | 0xFF00..=0xFFEF | 0x20000..=0x2FFFF)
}

/// (CJK chars, all non-space chars) of `s`.
pub fn cjk_counts(s: &str) -> (usize, usize) {
    let mut cjk = 0;
    let mut all = 0;
    for c in s.chars() {
        if c.is_whitespace() {
            continue;
        }
        all += 1;
        if is_cjk(c) {
            cjk += 1;
        }
    }
    (cjk, all)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rpr(xml: &str) -> RPr {
        let x = format!(r#"<w:rPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{xml}</w:rPr>"#);
        let doc = roxmltree::Document::parse(&x).unwrap();
        RPr::parse(doc.root_element(), &Theme::default())
    }

    #[test]
    fn overlay_keeps_unstated() {
        let mut base = rpr(r#"<w:b/><w:sz w:val="22"/><w:color w:val="FF0000"/>"#);
        base.overlay(&rpr(r#"<w:b w:val="0"/><w:color w:val="auto"/>"#));
        assert_eq!(base.b, Some(false));
        assert_eq!(base.size, Some(22.0));
        assert_eq!(base.color, Some(None));
    }

    #[test]
    fn theme_font_and_color() {
        let r = rpr(r#"<w:rFonts w:asciiTheme="minorHAnsi" w:eastAsia="宋体"/><w:color w:val="auto" w:themeColor="accent1"/>"#);
        assert_eq!(r.font_ascii.as_deref(), Some("Calibri"));
        assert_eq!(r.font_ea.as_deref(), Some("宋体"));
        assert_eq!(r.color, Some(Some(Rgb(0x44, 0x72, 0xC4))));
    }
}
