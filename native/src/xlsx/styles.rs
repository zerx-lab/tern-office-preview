//! `xl/styles.xml`: fonts, fills, borders, number formats and the cell formats (`cellXfs`)
//! that cells point at, each boiled down once to a class string.

use super::numfmt::{self, Format};
use crate::color::{self, Rgb, Theme};
use crate::css::{self, Border, Line};
use crate::xml::{self, child, elems, is};
use std::collections::HashMap;

/// What a color attribute needs to resolve to RGB.
pub struct ColorCtx<'a> {
    pub theme: &'a Theme,
    pub indexed: &'a [Rgb],
}

/// A SpreadsheetML color (`rgb` / `theme` / `indexed` + `tint`); `None` for `auto`.
pub fn color_of(get: &dyn Fn(&str) -> Option<String>, ctx: &ColorCtx) -> Option<Rgb> {
    let base = if let Some(rgb) = get("rgb") {
        Rgb::hex(&rgb)
    } else if let Some(t) = get("theme") {
        t.trim().parse::<usize>().ok().and_then(|i| ctx.theme.excel(i))
    } else if let Some(i) = get("indexed") {
        i.trim().parse::<usize>().ok().and_then(|i| color::indexed(i, ctx.indexed))
    } else {
        None
    }?;
    match get("tint").and_then(|t| t.trim().parse::<f64>().ok()) {
        Some(t) if t != 0.0 => Some(base.tint(t)),
        _ => Some(base),
    }
}

fn on(v: Option<String>) -> bool {
    !matches!(v.as_deref(), Some("0") | Some("false") | Some("off"))
}

#[derive(Clone, Default, Debug)]
pub struct Font {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    /// 0 none, 1 single, 2 double.
    pub underline: u8,
    pub size: Option<f64>,
    pub color: Option<Rgb>,
    pub name: Option<String>,
    /// 0 baseline, 1 superscript, 2 subscript.
    pub vert: u8,
}

impl Font {
    /// Applies one child element of `<font>` / `<rPr>` (`b`, `i`, `u`, `sz`, `color`, …).
    pub fn apply(&mut self, tag: &str, get: &dyn Fn(&str) -> Option<String>, ctx: &ColorCtx) {
        match tag {
            "b" => self.bold = on(get("val")),
            "i" => self.italic = on(get("val")),
            "strike" => self.strike = on(get("val")),
            "u" => {
                self.underline = match get("val").as_deref() {
                    None | Some("single") | Some("singleAccounting") => 1,
                    Some("double") | Some("doubleAccounting") => 2,
                    _ => 0,
                }
            }
            "sz" => self.size = get("val").and_then(|v| v.trim().parse().ok()),
            "color" => self.color = color_of(get, ctx),
            "name" | "rFont" => self.name = get("val"),
            "vertAlign" => {
                self.vert = match get("val").as_deref() {
                    Some("superscript") => 1,
                    Some("subscript") => 2,
                    _ => 0,
                }
            }
            _ => {}
        }
    }

    fn size_class(&self) -> String {
        css::size(self.size.unwrap_or(11.0) * 2.0).unwrap_or_else(|| "f22".into())
    }

    fn family_class(&self) -> Option<String> {
        self.name.as_deref().and_then(css::font_class)
    }

    fn decoration(&self, out: &mut String) {
        let c = match (self.underline, self.strike) {
            (0, false) => return,
            (1, false) => "u",
            (2, _) => "uu",
            (0, true) => "s",
            _ => "us",
        };
        push_class(out, c);
    }

    /// The classes of a run of rich text: complete (each run restates its font).
    pub fn run_class(&self) -> String {
        let mut s = String::new();
        push_class(&mut s, if self.bold { "b" } else { "nb" });
        if self.italic {
            push_class(&mut s, "i");
        }
        self.decoration(&mut s);
        if self.size.is_some() {
            push_class(&mut s, &self.size_class());
        }
        if let Some(f) = self.family_class() {
            push_class(&mut s, &f);
        }
        if let Some(c) = self.color {
            push_class(&mut s, &css::fg(c));
        }
        match self.vert {
            1 => push_class(&mut s, "sup"),
            2 => push_class(&mut s, "sub"),
            _ => {}
        }
        s
    }
}

pub fn push_class(out: &mut String, c: &str) {
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(c);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HAlign {
    General,
    Left,
    Center,
    Right,
    Justify,
}

pub const LEFT: usize = 0;
pub const RIGHT: usize = 1;
pub const TOP: usize = 2;
pub const BOTTOM: usize = 3;

/// One cell format, pre-digested.
#[derive(Clone, Debug)]
pub struct Xf {
    /// Font (but color), fill, vertical alignment, wrapping and explicit horizontal alignment.
    pub class: String,
    /// Font color class, dropped when a number format supplies its own color.
    pub fg: Option<String>,
    /// Index into `Styles::formats`.
    pub fmt: usize,
    pub halign: HAlign,
    pub wrap: bool,
    pub fill: bool,
    /// Left, right, top, bottom.
    pub borders: [Option<Border>; 4],
    pub any_border: bool,
    pub ind_left: Option<String>,
    pub ind_right: Option<String>,
    /// Font size in px, for estimating text width.
    pub font_px: f64,
}

impl Xf {
    fn plain() -> Xf {
        Xf {
            class: String::new(),
            fg: None,
            fmt: 0,
            halign: HAlign::General,
            wrap: false,
            fill: false,
            borders: [None; 4],
            any_border: false,
            ind_left: None,
            ind_right: None,
            font_px: 14.67,
        }
    }

    /// Whether an empty cell with this format draws anything (fill or a border).
    pub fn visible(&self) -> bool {
        self.fill || self.any_border
    }
}

pub struct Styles {
    pub xfs: Vec<Xf>,
    pub formats: Vec<Format>,
    /// Classes for the `<table>`: the workbook's default font where it differs from the sheet's.
    pub table_class: String,
    pub indexed: Vec<Rgb>,
}

impl Styles {
    pub fn parse(xml_text: Option<&str>, theme: &Theme) -> Styles {
        let mut st = Styles {
            xfs: vec![Xf::plain()],
            formats: vec![Format::general()],
            table_class: String::new(),
            indexed: Vec::new(),
        };
        let Some(text) = xml_text else { return st };
        let Ok(doc) = roxmltree::Document::parse_with_options(text, xml::OPTS) else { return st };
        let root = doc.root_element();

        if let Some(ic) = child(root, "x", "colors").and_then(|c| child(c, "x", "indexedColors")) {
            st.indexed = elems(ic).filter_map(|n| n.attribute("rgb").and_then(Rgb::hex)).collect();
        }
        let ctx = ColorCtx { theme, indexed: &st.indexed };
        let attr_get = getter;

        let mut custom: HashMap<u32, String> = HashMap::new();
        if let Some(nf) = child(root, "x", "numFmts") {
            for n in elems(nf).filter(|n| is(*n, "x", "numFmt")) {
                if let (Some(id), Some(code)) = (xml::num::<u32>(n.attribute("numFmtId")), n.attribute("formatCode")) {
                    custom.insert(id, code.to_string());
                }
            }
        }

        let mut fonts: Vec<Font> = Vec::new();
        if let Some(fs) = child(root, "x", "fonts") {
            for f in elems(fs).filter(|n| is(*n, "x", "font")) {
                let mut font = Font::default();
                for p in elems(f) {
                    font.apply(p.tag_name().name(), &attr_get(p), &ctx);
                }
                fonts.push(font);
            }
        }

        let mut fills: Vec<Option<Rgb>> = Vec::new();
        if let Some(fs) = child(root, "x", "fills") {
            for f in elems(fs).filter(|n| is(*n, "x", "fill")) {
                fills.push(fill_color(f, &ctx));
            }
        }

        let mut borders: Vec<[Option<Border>; 4]> = Vec::new();
        if let Some(bs) = child(root, "x", "borders") {
            for b in elems(bs).filter(|n| is(*n, "x", "border")) {
                let side = |names: [&str; 2]| {
                    elems(b).find(|n| names.contains(&n.tag_name().name())).and_then(|n| border_of(n, &ctx))
                };
                borders.push([
                    side(["left", "start"]),
                    side(["right", "end"]),
                    side(["top", "top"]),
                    side(["bottom", "bottom"]),
                ]);
            }
        }

        let default_font = fonts.first().cloned().unwrap_or_default();
        let def_size = default_font.size_class();
        let def_family = default_font.family_class();
        if def_size != "f22" {
            push_class(&mut st.table_class, &def_size);
        }
        if let Some(f) = &def_family {
            push_class(&mut st.table_class, f);
        }

        let mut by_code: HashMap<String, usize> = HashMap::new();
        by_code.insert("General".into(), 0);

        let Some(xfs) = child(root, "x", "cellXfs") else { return st };
        st.xfs.clear();
        for x in elems(xfs).filter(|n| is(*n, "x", "xf")).take(65_000) {
            let idx = |name: &str| xml::num::<usize>(x.attribute(name)).unwrap_or(0);
            let mut xf = Xf::plain();
            // number format
            let id = xml::num::<u32>(x.attribute("numFmtId")).unwrap_or(0);
            let code = custom.get(&id).map(|s| s.as_str()).or_else(|| numfmt::builtin(id)).unwrap_or("General");
            xf.fmt = match by_code.get(code) {
                Some(&i) => i,
                None => {
                    st.formats.push(numfmt::parse(code));
                    by_code.insert(code.to_string(), st.formats.len() - 1);
                    st.formats.len() - 1
                }
            };
            // font
            let font = fonts.get(idx("fontId")).unwrap_or(&default_font);
            let mut class = String::new();
            if font.bold {
                push_class(&mut class, "b");
            }
            if font.italic {
                push_class(&mut class, "i");
            }
            font.decoration(&mut class);
            let sc = font.size_class();
            if sc != def_size {
                push_class(&mut class, &sc);
            }
            let fam = font.family_class();
            if fam != def_family {
                if let Some(f) = &fam {
                    push_class(&mut class, f);
                }
            }
            match font.vert {
                1 => push_class(&mut class, "sup"),
                2 => push_class(&mut class, "sub"),
                _ => {}
            }
            xf.font_px = font.size.unwrap_or(11.0) * 4.0 / 3.0;
            xf.fg = font.color.filter(|c| *c != Rgb::BLACK).map(css::fg);
            // fill
            if let Some(Some(c)) = fills.get(idx("fillId")) {
                push_class(&mut class, &css::bg(*c));
                xf.fill = true;
            }
            // borders
            if let Some(b) = borders.get(idx("borderId")) {
                xf.borders = *b;
                xf.any_border = b.iter().any(|s| s.is_some());
            }
            // alignment
            if let Some(al) = child(x, "x", "alignment") {
                xf.halign = match al.attribute("horizontal") {
                    Some("left") => HAlign::Left,
                    Some("center") | Some("centerContinuous") => HAlign::Center,
                    Some("right") => HAlign::Right,
                    Some("justify") | Some("distributed") => HAlign::Justify,
                    _ => HAlign::General,
                };
                match al.attribute("vertical") {
                    Some("top") => push_class(&mut class, "vt"),
                    Some("center") => push_class(&mut class, "vm"),
                    _ => {}
                }
                if matches!(al.attribute("wrapText"), Some("1") | Some("true")) {
                    xf.wrap = true;
                    push_class(&mut class, "ww");
                }
                match xf.halign {
                    HAlign::Left => push_class(&mut class, "al"),
                    HAlign::Center => push_class(&mut class, "ac"),
                    HAlign::Right => push_class(&mut class, "ar"),
                    HAlign::Justify => push_class(&mut class, "aj"),
                    HAlign::General => {}
                }
                let indent = xml::num::<u32>(al.attribute("indent")).unwrap_or(0);
                if indent > 0 {
                    xf.ind_left = css::indent_level(indent);
                    xf.ind_right = css::pad_right(indent as f64 * 9.0 + 3.0);
                }
            }
            xf.class = class;
            st.xfs.push(xf);
        }
        if st.xfs.is_empty() {
            st.xfs.push(Xf::plain());
        }
        st
    }

    pub fn xf(&self, i: usize) -> &Xf {
        self.xfs.get(i).unwrap_or(&self.xfs[0])
    }
}

/// An attribute lookup over an element, in the shape `Font::apply` / `color_of` want.
fn getter<'a, 'i: 'a>(n: roxmltree::Node<'a, 'i>) -> impl Fn(&str) -> Option<String> + 'a {
    move |k: &str| n.attribute(k).map(String::from)
}

/// The fill a `<fill>` paints: solid / any pattern → its foreground, gradient → first stop.
fn fill_color(f: roxmltree::Node<'_, '_>, ctx: &ColorCtx) -> Option<Rgb> {
    let get = getter;
    if let Some(p) = child(f, "x", "patternFill") {
        let t = p.attribute("patternType").unwrap_or("solid");
        if t == "none" || t == "gray125" {
            return None;
        }
        return child(p, "x", "fgColor").and_then(|c| color_of(&get(c), ctx));
    }
    if let Some(g) = child(f, "x", "gradientFill") {
        let stop = child(g, "x", "stop")?;
        return child(stop, "x", "color").and_then(|c| color_of(&get(c), ctx));
    }
    None
}

fn border_of(n: roxmltree::Node<'_, '_>, ctx: &ColorCtx) -> Option<Border> {
    let line = match n.attribute("style")? {
        "none" => return None,
        "thin" | "hair" => Line::Thin,
        "medium" => Line::Medium,
        "thick" => Line::Thick,
        "double" => Line::Double,
        "dotted" => Line::Dotted,
        "dashed" | "dashDot" | "dashDotDot" | "mediumDashed" | "mediumDashDot" | "mediumDashDotDot"
        | "slantDashDot" => Line::Dashed,
        _ => Line::Thin,
    };
    let color = child(n, "x", "color").and_then(|c| color_of(&|k: &str| c.attribute(k).map(String::from), ctx));
    Some(Border { line, color })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<numFmts count="1"><numFmt numFmtId="164" formatCode="0.0&quot;x&quot;"/></numFmts>
<fonts count="4">
 <font><sz val="11"/><color theme="1"/><name val="Calibri"/></font>
 <font><b/><i/><u val="double"/><strike/><sz val="14"/><color rgb="FFFF0000"/><name val="宋体"/></font>
 <font><sz val="10"/><color theme="4" tint="0.4"/><name val="Arial"/><u/></font>
 <font><b val="0"/><sz val="11"/><color indexed="10"/><vertAlign val="superscript"/></font>
</fonts>
<fills count="4">
 <fill><patternFill patternType="none"/></fill>
 <fill><patternFill patternType="gray125"/></fill>
 <fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/><bgColor indexed="64"/></patternFill></fill>
 <fill><gradientFill degree="90"><stop position="0"><color rgb="FF00FF00"/></stop><stop position="1"><color rgb="FF0000FF"/></stop></gradientFill></fill>
</fills>
<borders count="3">
 <border><left/><right/><top/><bottom/><diagonal/></border>
 <border><left style="thin"><color auto="1"/></left><right style="medium"><color rgb="FF808080"/></right><top style="dashed"/><bottom style="double"/></border>
 <border><left style="hair"/><right style="mediumDashed"/><top style="thick"/><bottom style="dotted"/></border>
</borders>
<cellXfs count="5">
 <xf numFmtId="0" fontId="0" fillId="0" borderId="0"/>
 <xf numFmtId="164" fontId="1" fillId="2" borderId="1" applyAlignment="1"><alignment horizontal="center" vertical="center" wrapText="1"/></xf>
 <xf numFmtId="14" fontId="2" fillId="3" borderId="2"><alignment horizontal="left" indent="2" vertical="top"/></xf>
 <xf numFmtId="9" fontId="3" fillId="0" borderId="0"/>
 <xf numFmtId="164" fontId="0" fillId="1" borderId="0"/>
</cellXfs>
</styleSheet>"#;

    fn styles() -> Styles {
        Styles::parse(Some(STYLES), &Theme::default())
    }

    #[test]
    fn formats_are_shared_and_resolved() {
        let s = styles();
        assert_eq!(s.xfs.len(), 5);
        assert_eq!(s.xfs[1].fmt, s.xfs[4].fmt);
        let mut out = String::new();
        s.formats[s.xfs[1].fmt].number(2.5, false, &mut out);
        assert_eq!(out, "2.5x");
        out.clear();
        s.formats[s.xfs[2].fmt].number(45000.0, false, &mut out);
        assert_eq!(out, "2023/3/15");
        assert!(s.formats[s.xfs[0].fmt].is_general());
    }

    #[test]
    fn font_fill_alignment_classes() {
        let s = styles();
        let x = &s.xfs[1];
        assert_eq!(x.class, "b i uu f28 ffsong k770 vm ww ac");
        assert_eq!(x.fg.as_deref(), Some("c700"));
        assert!(x.fill && x.wrap && x.any_border);
        assert_eq!(x.halign, HAlign::Center);
        assert!((x.font_px - 18.67).abs() < 0.01);
        // Plain default cell: nothing at all.
        assert_eq!(s.xfs[0].class, "");
        assert!(s.xfs[0].fg.is_none());
        assert!(!s.xfs[0].visible());
    }

    #[test]
    fn theme_tint_indexed_and_gradient() {
        let s = styles();
        let x = &s.xfs[2];
        assert_eq!(x.class, "u f20 k070 vt al");
        let want = css::fg(Theme::default().excel(4).unwrap().tint(0.4));
        assert_eq!(x.fg.as_deref(), Some(want.as_str()));
        assert_eq!(x.ind_left.as_deref(), Some("xi2"));
        assert!(x.ind_right.is_some());
        // Third xf: font 3 → superscript, indexed color 10 (red), bold off.
        let y = &s.xfs[3];
        assert_eq!(y.class, "sup");
        assert_eq!(y.fg.as_deref(), Some("c700"));
        // gray125 is not a fill.
        assert!(!s.xfs[4].fill);
    }

    #[test]
    fn border_mapping() {
        let s = styles();
        let b = s.xfs[1].borders;
        assert_eq!(b[LEFT], Some(Border { line: Line::Thin, color: None }));
        assert_eq!(b[RIGHT], Some(Border { line: Line::Medium, color: Some(Rgb(0x80, 0x80, 0x80)) }));
        assert_eq!(b[TOP], Some(Border { line: Line::Dashed, color: None }));
        assert_eq!(b[BOTTOM], Some(Border { line: Line::Double, color: None }));
        let c = s.xfs[2].borders;
        assert_eq!(c[LEFT].map(|b| b.line), Some(Line::Thin));
        assert_eq!(c[RIGHT].map(|b| b.line), Some(Line::Dashed));
        assert_eq!(c[TOP].map(|b| b.line), Some(Line::Thick));
        assert_eq!(c[BOTTOM].map(|b| b.line), Some(Line::Dotted));
        assert!(!s.xfs[0].any_border);
    }

    #[test]
    fn default_font_goes_on_the_table() {
        let xml = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<fonts><font><sz val="10"/><name val="宋体"/></font><font><sz val="10"/><name val="宋体"/><b/></font><font><sz val="12"/><name val="Arial"/></font></fonts>
<fills><fill/></fills><borders><border/></borders>
<cellXfs><xf fontId="0"/><xf fontId="1"/><xf fontId="2"/></cellXfs></styleSheet>"#;
        let s = Styles::parse(Some(xml), &Theme::default());
        assert_eq!(s.table_class, "f20 ffsong");
        assert_eq!(s.xfs[0].class, "");
        assert_eq!(s.xfs[1].class, "b");
        assert_eq!(s.xfs[2].class, "f24 ffh");
    }

    #[test]
    fn custom_indexed_palette() {
        let xml = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<colors><indexedColors><rgbColor rgb="FF000000"/><rgbColor rgb="FFFFFFFF"/><rgbColor rgb="FF123456"/></indexedColors></colors>
<fonts><font><color indexed="2"/></font></fonts><fills><fill/></fills><borders><border/></borders>
<cellXfs><xf fontId="0"/></cellXfs></styleSheet>"#;
        let s = Styles::parse(Some(xml), &Theme::default());
        assert_eq!(s.xfs[0].fg.as_deref(), Some(css::fg(Rgb(0x12, 0x34, 0x56)).as_str()));
    }

    #[test]
    fn missing_styles_still_work() {
        let s = Styles::parse(None, &Theme::default());
        assert_eq!(s.xfs.len(), 1);
        assert!(s.xf(99).class.is_empty());
    }
}
