//! `styles.xml`: docDefaults, the paragraph / character style chains (`basedOn`) and table
//! styles with their conditional formatting.

use super::props::{shd_fill, Bdrs, PPr, RPr};
use crate::color::{Rgb, Theme};
use crate::xml::{self, attr, child, children, elems, is, num, wval};
use roxmltree::Node;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Para,
    Char,
    Table,
    Other,
}

/// One block of table-style formatting (the whole table, or one `tblStylePr` condition).
#[derive(Clone, Default, Debug)]
pub struct Cond {
    pub rpr: RPr,
    pub ppr: PPr,
    pub shd: Option<Option<Rgb>>,
    pub borders: Bdrs,
}

impl Cond {
    fn overlay(&mut self, o: &Cond) {
        self.rpr.overlay(&o.rpr);
        self.ppr.overlay(&o.ppr);
        if o.shd.is_some() {
            self.shd = o.shd;
        }
        self.borders.overlay(&o.borders);
    }
}

#[derive(Clone, Default, Debug)]
pub struct TblStyle {
    /// `tblPr/tblBorders` of the style (outer edges and inside lines).
    pub borders: Bdrs,
    pub whole: Cond,
    pub conds: HashMap<String, Cond>,
    pub row_band: usize,
    pub col_band: usize,
}

#[derive(Clone, Debug)]
struct Def {
    kind: Kind,
    name: String,
    based_on: Option<String>,
    ppr: PPr,
    rpr: RPr,
    tbl: TblStyle,
    is_default: bool,
}

#[derive(Default)]
pub struct Styles {
    pub def_ppr: PPr,
    pub def_rpr: RPr,
    resolved_p: HashMap<String, Rc<(PPr, RPr)>>,
    resolved_c: HashMap<String, Rc<RPr>>,
    resolved_t: HashMap<String, Rc<TblStyle>>,
    /// Style id → heading level (1–9), from the outline level or the style's name.
    headings: HashMap<String, u8>,
    default_p: Option<String>,
    default_t: Option<String>,
}

/// `heading 1` / `Heading1` / `标题 1` → 1.
fn heading_from_name(name: &str) -> Option<u8> {
    let n = name.trim().to_lowercase();
    let rest = n.strip_prefix("heading").or_else(|| n.strip_prefix("标题"))?;
    let d: u8 = rest.trim().parse().ok()?;
    (1..=9).contains(&d).then_some(d)
}

fn parse_cond(n: Node, theme: &Theme) -> Cond {
    let mut c = Cond::default();
    for e in elems(n) {
        match xml::qname(e).as_str() {
            "w:rPr" => c.rpr = RPr::parse(e, theme),
            "w:pPr" => c.ppr = PPr::parse(e, theme),
            "w:tcPr" => {
                if let Some(b) = child(e, "w", "tcBorders") {
                    c.borders = Bdrs::parse(b, theme);
                }
                if let Some(s) = child(e, "w", "shd") {
                    if let Some(f) = shd_fill(s, theme) {
                        c.shd = Some(f);
                    }
                }
            }
            "w:tblPr" => {
                // Conditional `tblPr` can carry borders too (rare); fold them in.
                if let Some(b) = child(e, "w", "tblBorders") {
                    c.borders.overlay(&Bdrs::parse(b, theme));
                }
            }
            _ => {}
        }
    }
    c
}

impl Styles {
    pub fn parse(xml_text: Option<&str>, theme: &Theme) -> Styles {
        let mut s = Styles::default();
        let Some(text) = xml_text else { return s };
        let Ok(doc) = roxmltree::Document::parse_with_options(text, xml::OPTS) else { return s };
        let mut defs: HashMap<String, Def> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for n in doc.root_element().children().filter(|c| c.is_element()) {
            if is(n, "w", "docDefaults") {
                if let Some(r) = child(n, "w", "rPrDefault").and_then(|d| child(d, "w", "rPr")) {
                    s.def_rpr = RPr::parse(r, theme);
                }
                if let Some(p) = child(n, "w", "pPrDefault").and_then(|d| child(d, "w", "pPr")) {
                    s.def_ppr = PPr::parse(p, theme);
                }
            } else if is(n, "w", "style") {
                let Some(id) = attr(n, "w", "styleId") else { continue };
                let kind = match attr(n, "w", "type") {
                    Some("paragraph") => Kind::Para,
                    Some("character") => Kind::Char,
                    Some("table") => Kind::Table,
                    _ => Kind::Other,
                };
                let name = child(n, "w", "name").and_then(wval).unwrap_or("").to_string();
                let based_on = child(n, "w", "basedOn").and_then(wval).map(str::to_string);
                let mut d = Def {
                    kind,
                    name,
                    based_on,
                    ppr: PPr::default(),
                    rpr: RPr::default(),
                    tbl: TblStyle::default(),
                    is_default: xml::on_off(attr(n, "w", "default")) && attr(n, "w", "default").is_some(),
                };
                if let Some(p) = child(n, "w", "pPr") {
                    d.ppr = PPr::parse(p, theme);
                }
                if let Some(r) = child(n, "w", "rPr") {
                    d.rpr = RPr::parse(r, theme);
                }
                if kind == Kind::Table {
                    d.tbl.whole = parse_cond(n, theme);
                    if let Some(tp) = child(n, "w", "tblPr") {
                        if let Some(b) = child(tp, "w", "tblBorders") {
                            d.tbl.borders = Bdrs::parse(b, theme);
                        }
                        d.tbl.row_band = num(child(tp, "w", "tblStyleRowBandSize").and_then(wval)).unwrap_or(1);
                        d.tbl.col_band = num(child(tp, "w", "tblStyleColBandSize").and_then(wval)).unwrap_or(1);
                    }
                    for sp in children(n, "w", "tblStylePr") {
                        if let Some(t) = attr(sp, "w", "type") {
                            d.tbl.conds.insert(t.to_string(), parse_cond(sp, theme));
                        }
                    }
                }
                order.push(id.to_string());
                defs.insert(id.to_string(), d);
            }
        }

        for id in &order {
            let d = &defs[id];
            match d.kind {
                Kind::Para => {
                    if d.is_default && s.default_p.is_none() {
                        s.default_p = Some(id.clone());
                    }
                    let (mut ppr, mut rpr) = (PPr::default(), RPr::default());
                    for sid in chain(&defs, id).iter().rev() {
                        let sd = &defs[sid];
                        ppr.overlay(&sd.ppr);
                        rpr.overlay(&sd.rpr);
                    }
                    // Heading level: outline level on the chain, else the name of the chain's styles.
                    let level = ppr.outline.filter(|l| *l < 9).map(|l| l + 1).or_else(|| {
                        chain(&defs, id).iter().find_map(|sid| heading_from_name(&defs[sid].name)).or_else(|| {
                            chain(&defs, id).iter().find_map(|sid| heading_from_name(sid))
                        })
                    });
                    if ppr.outline == Some(9) {
                        // outlineLvl 9 = body text.
                    } else if let Some(l) = level {
                        s.headings.insert(id.clone(), l);
                    }
                    s.resolved_p.insert(id.clone(), Rc::new((ppr, rpr)));
                }
                Kind::Char => {
                    let mut rpr = RPr::default();
                    for sid in chain(&defs, id).iter().rev() {
                        rpr.overlay(&defs[sid].rpr);
                    }
                    s.resolved_c.insert(id.clone(), Rc::new(rpr));
                }
                Kind::Table => {
                    if d.is_default && s.default_t.is_none() {
                        s.default_t = Some(id.clone());
                    }
                    let mut t = TblStyle { row_band: 1, col_band: 1, ..Default::default() };
                    for sid in chain(&defs, id).iter().rev() {
                        let sd = &defs[sid];
                        t.borders.overlay(&sd.tbl.borders);
                        t.whole.overlay(&sd.tbl.whole);
                        // A table style's own pPr / rPr act on the whole table.
                        t.whole.ppr.overlay(&sd.ppr);
                        t.whole.rpr.overlay(&sd.rpr);
                        for (k, c) in &sd.tbl.conds {
                            t.conds.entry(k.clone()).or_default().overlay(c);
                        }
                        t.row_band = sd.tbl.row_band.max(1);
                        t.col_band = sd.tbl.col_band.max(1);
                    }
                    s.resolved_t.insert(id.clone(), Rc::new(t));
                }
                Kind::Other => {}
            }
        }
        s
    }

    /// Paragraph style chain properties (no docDefaults), for `style` or the default style.
    pub fn para(&self, style: Option<&str>) -> Option<Rc<(PPr, RPr)>> {
        let id = style.filter(|s| self.resolved_p.contains_key(*s)).or(self.default_p.as_deref())?;
        self.resolved_p.get(id).cloned()
    }

    pub fn default_para(&self) -> Option<&str> {
        self.default_p.as_deref()
    }

    pub fn char(&self, style: &str) -> Option<Rc<RPr>> {
        self.resolved_c.get(style).cloned()
    }

    pub fn table(&self, style: Option<&str>) -> Option<Rc<TblStyle>> {
        let id = style.filter(|s| self.resolved_t.contains_key(*s)).or(self.default_t.as_deref())?;
        self.resolved_t.get(id).cloned()
    }

    /// Heading level (1–9) of a paragraph style, if it is a heading style.
    pub fn heading(&self, style: Option<&str>) -> Option<u8> {
        let id = style.filter(|s| self.resolved_p.contains_key(*s)).or(self.default_p.as_deref())?;
        self.headings.get(id).copied()
    }
}

/// `id` and its `basedOn` ancestors, leaf first (cycle-safe).
fn chain(defs: &HashMap<String, Def>, id: &str) -> Vec<String> {
    let mut out = vec![id.to_string()];
    let mut cur = id;
    while let Some(b) = defs.get(cur).and_then(|d| d.based_on.as_deref()) {
        if out.len() > 24 || out.iter().any(|o| o == b) || !defs.contains_key(b) {
            break;
        }
        out.push(b.to_string());
        cur = b;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

    #[test]
    fn style_chain_overlays_base_to_leaf() {
        let xml = format!(
            r#"<w:styles {NS}>
            <w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="21"/></w:rPr></w:rPrDefault></w:docDefaults>
            <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/>
              <w:pPr><w:jc w:val="both"/></w:pPr><w:rPr><w:sz w:val="24"/></w:rPr></w:style>
            <w:style w:type="paragraph" w:styleId="H1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/>
              <w:pPr><w:keepNext/><w:spacing w:before="240"/></w:pPr><w:rPr><w:b/></w:rPr></w:style>
            <w:style w:type="paragraph" w:styleId="H1b"><w:name w:val="My"/><w:basedOn w:val="H1"/>
              <w:rPr><w:sz w:val="32"/></w:rPr></w:style>
            <w:style w:type="paragraph" w:styleId="Out"><w:name w:val="Out"/><w:pPr><w:outlineLvl w:val="1"/></w:pPr></w:style>
            </w:styles>"#
        );
        let s = Styles::parse(Some(&xml), &Theme::default());
        assert_eq!(s.def_rpr.size, Some(21.0));
        let r = s.para(Some("H1b")).unwrap();
        assert_eq!(r.0.jc, Some(super::super::props::Jc::Both));
        assert_eq!(r.0.before, Some(240.0));
        assert_eq!(r.1.b, Some(true));
        assert_eq!(r.1.size, Some(32.0));
        // No pStyle → the default style.
        assert_eq!(s.para(None).unwrap().1.size, Some(24.0));
        assert_eq!(s.heading(Some("H1")), Some(1));
        assert_eq!(s.heading(Some("H1b")), Some(1));
        assert_eq!(s.heading(Some("Out")), Some(2));
        assert_eq!(s.heading(Some("Normal")), None);
    }

    #[test]
    fn table_style_conditions_merge_along_chain() {
        let xml = format!(
            r#"<w:styles {NS}>
            <w:style w:type="table" w:styleId="A"><w:name w:val="A"/>
              <w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4"/></w:tblBorders></w:tblPr>
              <w:tblStylePr w:type="firstRow"><w:rPr><w:b/></w:rPr><w:tcPr><w:shd w:val="clear" w:fill="4472C4"/></w:tcPr></w:tblStylePr>
            </w:style>
            <w:style w:type="table" w:styleId="B"><w:name w:val="B"/><w:basedOn w:val="A"/>
              <w:tblStylePr w:type="firstRow"><w:rPr><w:color w:val="FFFFFF"/></w:rPr></w:tblStylePr>
            </w:style></w:styles>"#
        );
        let s = Styles::parse(Some(&xml), &Theme::default());
        let t = s.table(Some("B")).unwrap();
        let f = &t.conds["firstRow"];
        assert_eq!(f.rpr.b, Some(true));
        assert_eq!(f.rpr.color, Some(Some(Rgb(255, 255, 255))));
        assert_eq!(f.shd, Some(Some(Rgb(0x44, 0x72, 0xC4))));
        assert!(t.borders.top.is_some());
    }
}
