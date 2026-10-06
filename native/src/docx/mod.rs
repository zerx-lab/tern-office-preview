//! .docx rendering: the body becomes `p` / `table` / `div` nodes with utility classes from
//! `css.rs`, split into parts at top-level block boundaries.

mod body;
mod drawing;
mod math;
mod numbering;
mod props;
mod styles;
mod table;

use crate::color::Theme;
use crate::css;
use crate::node::Node;
use crate::out::Out;
use crate::pkg::{Package, Rels};
use crate::xml::{self, attr, child, children, elems, is};
use numbering::Numbering;
use props::{PPr, RPr};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use roxmltree::Node as X;
use std::rc::Rc;
use styles::Styles;

/// Running `weight()` after which a part is closed.
const PART_BUDGET: usize = 4000;
/// A split prefers a "natural" boundary inside the last this-much of the budget.
const PREFER_TAIL: f64 = 0.25;
/// Caps on the page margins, px.
const MAX_MARGIN: f64 = 96.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Instr,
    Result,
}

/// A complex field being walked (`fldChar` begin … end).
pub struct Field {
    pub phase: Phase,
    pub instr: String,
    pub href: Option<String>,
}

/// Table-style paragraph / run formatting of the cell being rendered.
pub struct TblLayer {
    pub ppr: PPr,
    pub rpr: RPr,
}

/// A referenced footnote / endnote, in order of first reference.
pub struct NoteRef {
    pub endnote: bool,
    pub id: String,
}

pub struct OutlineItem {
    pub text: String,
    pub level: u8,
    pub block: usize,
}

/// Page geometry of a section, px.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Geom {
    pub w: f64,
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

impl Geom {
    pub const DEFAULT: Geom = Geom { w: 794.0, left: MAX_MARGIN, right: MAX_MARGIN, top: MAX_MARGIN, bottom: MAX_MARGIN };

    fn same(&self, o: &Geom) -> bool {
        let r = |v: f64| (v / 4.0).round();
        r(self.w) == r(o.w) && r(self.left) == r(o.left) && r(self.right) == r(o.right)
    }

    fn content_width(&self) -> f64 {
        (self.w - self.left - self.right).max(120.0)
    }

    fn class(&self) -> String {
        let mut s = String::from("pg");
        for c in [
            css::max_width(self.w),
            css::pad_left(self.left),
            css::pad_right(self.right),
            css::pad_top(self.top),
            css::pad_bottom(self.bottom),
        ]
        .into_iter()
        .flatten()
        {
            s.push(' ');
            s.push_str(&c);
        }
        s
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Section {
    pub geom: Geom,
    /// `w:type continuous`: starts on the same page as the previous section.
    pub continuous: bool,
}

pub struct Ctx<'a> {
    pub pkg: &'a mut Package,
    pub out: &'a mut Out,
    pub theme: Theme,
    pub styles: Styles,
    pub numbering: Numbering,
    /// Relationships of the part being rendered (document, header, footnotes…).
    pub rels: Rels,
    pub fields: Vec<Field>,
    pub notes: Vec<NoteRef>,
    pub outline: Vec<OutlineItem>,
    /// Index of the top-level block being produced (for the outline's part lookup).
    pub block: usize,
    pub outline_on: bool,
    /// Heading level (1–9) of the paragraph last produced at top level, 0 if none.
    pub heading: u8,
    /// Width available to content, px.
    pub avail: f64,
    pub tbl: Option<Rc<TblLayer>>,
    /// Text box nesting depth.
    pub depth: u32,
    /// Text put in front of the next paragraph (a footnote's number): (classes, text).
    pub prefix: Option<(String, String)>,
    pub counts: BTreeMap<&'static str, usize>,
}

impl Ctx<'_> {
    pub fn count(&mut self, what: &'static str) {
        *self.counts.entry(what).or_insert(0) += 1;
    }
}

/// A top-level piece of the body: one node with what the splitter needs to know.
pub struct Block {
    pub node: Node,
    pub weight: usize,
    pub brk: bool,
    pub heading: u8,
    pub sec: usize,
}

/// What `split` needs of a block.
#[derive(Clone, Copy)]
pub struct Piece {
    pub weight: usize,
    pub brk: bool,
    pub heading: u8,
}

/// Cuts `pieces` into consecutive ranges of about `budget` weight. A boundary is taken at the
/// last "natural" place (before a level 1–2 heading, after a page break) that lies in the final
/// `PREFER_TAIL` of the budget; otherwise right where the budget runs out.
pub fn split(pieces: &[Piece], budget: usize) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut j = 0;
    let mut cum = 0usize;
    let good = |i: usize| (1..=2).contains(&pieces[i].heading) || (i > 0 && pieces[i - 1].brk);
    while j < pieces.len() {
        if cum + pieces[j].weight > budget && j > start {
            let min = (budget as f64 * (1.0 - PREFER_TAIL)) as usize;
            let mut at = j;
            let mut before = cum;
            // Walk back from `j` looking for the last good boundary with enough weight before it.
            let mut i = j;
            while i > start {
                if before < min {
                    break;
                }
                if good(i) && i < j {
                    at = i;
                    break;
                }
                i -= 1;
                before -= pieces[i].weight;
            }
            ranges.push((start, at));
            start = at;
            cum = pieces[start..j].iter().map(|p| p.weight).sum();
            continue;
        }
        cum += pieces[j].weight;
        j += 1;
    }
    if start < pieces.len() {
        ranges.push((start, pieces.len()));
    }
    ranges
}

fn section_of(sect: X) -> Section {
    let mut g = Geom::DEFAULT;
    if let Some(sz) = child(sect, "w", "pgSz") {
        if let Some(w) = xml::twips(attr(sz, "w", "w")) {
            g.w = (w / 15.0).clamp(240.0, 2400.0);
        }
    }
    if let Some(m) = child(sect, "w", "pgMar") {
        let f = |name: &str, d: f64| xml::twips(attr(m, "w", name)).map_or(d, |t| (t / 15.0).clamp(0.0, MAX_MARGIN));
        g.left = f("left", g.left);
        g.right = f("right", g.right);
        g.top = f("top", g.top);
        g.bottom = f("bottom", g.bottom);
    }
    // Margins must leave room for text.
    let over = g.left + g.right - g.w * 0.45;
    if over > 0.0 {
        let k = (g.left + g.right - over) / (g.left + g.right);
        g.left *= k;
        g.right *= k;
    }
    let continuous = child(sect, "w", "type").and_then(xml::wval) == Some("continuous");
    Section { geom: g, continuous }
}

fn pbr() -> Node {
    Node::el("div").class("pbr")
}

pub fn has_content(n: &Node) -> bool {
    if n.kind == "image" || n.kind == "math" {
        return true;
    }
    if n.props.get("text").and_then(|t| t.as_str()).is_some_and(|t| t.chars().any(|c| !c.is_whitespace())) {
        return true;
    }
    n.children.iter().any(has_content)
}

fn core_title(pkg: &mut Package) -> String {
    let Some(text) = pkg.text("docProps/core.xml") else { return String::new() };
    let Ok(doc) = roxmltree::Document::parse_with_options(&text, xml::OPTS) else { return String::new() };
    doc.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "title")
        .map(|n| xml::text(n).trim().to_string())
        .unwrap_or_default()
}

fn rel_part(rels: &Rels, suffix: &str) -> Option<String> {
    rels.values().find(|r| !r.external && r.kind.ends_with(suffix)).map(|r| r.target.clone())
}

pub fn render(pkg: &mut Package, out: &mut Out) -> Result<Value, String> {
    let main = pkg.main().to_string();
    let text = pkg.text(&main).ok_or("找不到文档正文部件")?;
    let doc = roxmltree::Document::parse_with_options(&text, xml::OPTS).map_err(|e| format!("文档正文解析失败: {e}"))?;
    let rels = pkg.rels(&main)?;
    let theme = rel_part(&rels, "/theme").and_then(|p| pkg.text(&p)).map(|t| Theme::parse(&t)).unwrap_or_default();
    let styles_text = rel_part(&rels, "/styles").and_then(|p| pkg.text(&p));
    let numbering_text = rel_part(&rels, "/numbering").and_then(|p| pkg.text(&p));
    let styles = Styles::parse(styles_text.as_deref(), &theme);
    let numbering = Numbering::parse(numbering_text.as_deref(), &theme);
    let title = core_title(pkg);
    let body = child(doc.root_element(), "w", "body").ok_or("文档缺少正文（w:body）")?;

    // Sections: each paragraph-level sectPr closes one, the body's closes the last.
    let mut sections: Vec<Section> = Vec::new();
    for p in children(body, "w", "p") {
        if let Some(s) = child(p, "w", "pPr").and_then(|pp| child(pp, "w", "sectPr")) {
            sections.push(section_of(s));
        }
    }
    let body_sect = child(body, "w", "sectPr");
    sections.push(body_sect.map(|s| section_of(s)).unwrap_or(Section { geom: Geom::DEFAULT, continuous: false }));
    let first_geom = sections[0].geom;

    let mut cx = Ctx {
        pkg,
        out,
        theme,
        styles,
        numbering,
        rels: rels.clone(),
        fields: Vec::new(),
        notes: Vec::new(),
        outline: Vec::new(),
        block: 0,
        outline_on: true,
        heading: 0,
        avail: first_geom.content_width(),
        tbl: None,
        depth: 0,
        prefix: None,
        counts: BTreeMap::new(),
    };

    // Body.
    let mut blocks: Vec<Block> = Vec::new();
    let mut sec = 0usize;
    for c in elems(body) {
        if is(c, "w", "sectPr") {
            continue;
        }
        cx.avail = sections[sec.min(sections.len() - 1)].geom.content_width();
        cx.block = blocks.len();
        cx.heading = 0;
        let mut nodes = Vec::new();
        cx.block_child(c, &mut nodes);
        let mut heading = std::mem::take(&mut cx.heading);
        for node in nodes {
            let brk = node.props.get("class").and_then(|c| c.as_str()) == Some("pbr");
            let h = if brk { 0 } else { std::mem::take(&mut heading) };
            // Collapse consecutive breaks.
            if brk && blocks.last().is_none_or(|b| b.brk) {
                continue;
            }
            let weight = node.weight();
            blocks.push(Block { node, weight, brk, heading: h, sec });
        }
        if is(c, "w", "p") && child(c, "w", "pPr").and_then(|pp| child(pp, "w", "sectPr")).is_some() {
            sec += 1;
        }
    }
    let last_sec = sections.len() - 1;
    for b in &mut blocks {
        b.sec = b.sec.min(last_sec);
    }

    // Header / footer of the first section.
    let sect0 = child_first_sect(body);
    let mut head: Vec<Block> = Vec::new();
    let mut foot: Vec<Block> = Vec::new();
    cx.outline_on = false;
    cx.avail = first_geom.content_width();
    if let Some(s) = sect0 {
        if let Some(nodes) = cx.header_footer(s, "headerReference") {
            let n = Node::el("div").class("hf");
            let n = nodes.into_iter().fold(n, |n, c| n.child(c));
            head.push(mk_block(n, 0));
            head.push(mk_block(Node::el("div").class("hfr"), 0));
        }
        if let Some(nodes) = cx.header_footer(s, "footerReference") {
            let n = Node::el("div").class("hf");
            let n = nodes.into_iter().fold(n, |n, c| n.child(c));
            foot.push(mk_block(Node::el("div").class("hfr"), last_sec));
            foot.push(mk_block(n, last_sec));
        }
    }
    // Footnotes and endnotes, in chunks the splitter can place.
    let mut notes_blocks: Vec<Block> = Vec::new();
    for chunk in cx.render_notes() {
        notes_blocks.push(mk_block(chunk, last_sec));
    }

    let hdr_n = head.len();
    let mut all: Vec<Block> = head;
    all.extend(blocks);
    all.extend(notes_blocks);
    all.extend(foot);

    let pieces: Vec<Piece> = all.iter().map(|b| Piece { weight: b.weight, brk: b.brk, heading: b.heading }).collect();
    let ranges = split(&pieces, PART_BUDGET);
    let mut block_part = vec![0usize; all.len()];
    for (pi, (s, e)) in ranges.iter().enumerate() {
        for slot in &mut block_part[*s..*e] {
            *slot = pi;
        }
    }

    // Assemble the parts.
    let mut slots: Vec<Option<Block>> = all.into_iter().map(Some).collect();
    let mut parts_meta = Vec::new();
    for (pi, (s, e)) in ranges.iter().enumerate() {
        let mut segs: Vec<(usize, Node)> = Vec::new(); // (section, pg)
        let mut prev_sec: Option<usize> = None;
        for slot in &mut slots[*s..*e] {
            let b = slot.take().expect("block used once");
            // A page break at a part edge is just the edge.
            if b.brk && segs.is_empty() {
                continue;
            }
            let g = sections[b.sec].geom;
            let new_pg = match segs.last() {
                None => true,
                Some((ps, _)) => !sections[*ps].geom.same(&g),
            };
            if new_pg {
                segs.push((b.sec, Node::el("div").class(&g.class())));
            } else if prev_sec.is_some_and(|p| p != b.sec) && !sections[b.sec].continuous && !b.brk {
                segs.last_mut().expect("segment").1.push(pbr());
            }
            prev_sec = Some(b.sec);
            segs.last_mut().expect("segment").1.push(b.node);
        }
        // Trailing break at the edge of a part.
        for (_, pg) in &mut segs {
            while pg.children.last().is_some_and(|c| c.props.get("class").and_then(|c| c.as_str()) == Some("pbr")) {
                pg.children.pop();
            }
        }
        if segs.is_empty() {
            segs.push((0, Node::el("div").class(&first_geom.class()).child(Node::el("p").text("\u{00A0}"))));
        }
        let mut root = Node::el("div").class("opv opd");
        for (_, pg) in segs {
            root.push(pg);
        }
        let name = format!("part-{pi}");
        cx.out.write_part(&name, root)?;
        parts_meta.push(json!({ "file": name, "label": (pi + 1).to_string() }));
    }
    if ranges.is_empty() {
        let root = Node::el("div")
            .class("opv opd")
            .child(Node::el("div").class(&first_geom.class()).child(Node::el("p").text("\u{00A0}")));
        cx.out.write_part("part-0", root)?;
        parts_meta.push(json!({ "file": "part-0", "label": "1" }));
    }

    let outline: Vec<Value> = cx
        .outline
        .iter()
        .take(3000)
        .map(|o| json!({ "text": o.text, "level": o.level, "part": block_part.get(o.block + hdr_n).copied().unwrap_or(0) }))
        .collect();

    let mut notes: Vec<String> = Vec::new();
    for (k, n) in &cx.counts {
        notes.push(match *k {
            "emf" => format!("含 {n} 个无法显示的 EMF/WMF 图片或公式对象"),
            "img" => format!("含 {n} 个无法显示的图片"),
            "chart" => format!("含 {n} 个图表（以占位符显示）"),
            "smartart" => format!("含 {n} 个 SmartArt 图形（以占位符显示）"),
            "shape" => format!("含 {n} 个图形/形状（未绘制）"),
            _ => continue,
        });
    }

    Ok(json!({
        "kind": "docx",
        "title": title,
        "parts": parts_meta,
        "outline": outline,
        "notes": notes,
    }))
}

fn mk_block(node: Node, sec: usize) -> Block {
    let weight = node.weight();
    Block { node, weight, brk: false, heading: 0, sec }
}

/// The `sectPr` of the first section: the first paragraph-level one, else the body's.
fn child_first_sect<'a, 'i>(body: X<'a, 'i>) -> Option<X<'a, 'i>> {
    for p in children(body, "w", "p") {
        if let Some(s) = child(p, "w", "pPr").and_then(|pp| child(pp, "w", "sectPr")) {
            return Some(s);
        }
    }
    child(body, "w", "sectPr")
}

#[cfg(test)]
mod tests;
