//! `xl/drawings/*.xml`: pictures anchored in a sheet and chart frames (charts are placeholders).

use super::sheet::col_letters;
use crate::node::{self, Node};
use crate::out::Out;
use crate::pkg::Package;
use crate::xml::{self, attr, child, is};
use roxmltree::Node as XNode;

#[derive(Debug, PartialEq)]
pub enum Kind {
    /// Picture part name in the package.
    Picture(String),
    Chart,
}

#[derive(Debug)]
pub struct Item {
    pub kind: Kind,
    pub name: String,
    pub alt: String,
    /// 0-based anchor cell.
    pub row: u32,
    pub col: u32,
    /// Display size in px, when the drawing says.
    pub size: Option<(f64, f64)>,
}

#[derive(Debug, Default)]
pub struct Drawing {
    pub items: Vec<Item>,
    /// Shapes / text boxes / connectors that are not drawn.
    pub shapes: usize,
}

const EMU_PER_PX: f64 = 9525.0;

fn int_child(n: XNode, name: &str) -> f64 {
    child(n, "xdr", name).and_then(|c| c.text()).and_then(|t| t.trim().parse().ok()).unwrap_or(0.0)
}

fn emu_attr(n: XNode, name: &str) -> Option<f64> {
    n.attribute(name).and_then(|v| v.trim().parse::<f64>().ok())
}

/// Parses drawing part `part`. `col_px` / `row_px` give the sheet's column widths and row heights
/// (px), to size two-cell anchored pictures that carry no explicit extent.
pub fn parse(pkg: &mut Package, part: &str, col_px: &dyn Fn(u32) -> f64, row_px: &dyn Fn(u32) -> f64) -> Drawing {
    let mut d = Drawing::default();
    let Some(text) = pkg.text(part) else { return d };
    let Ok(doc) = roxmltree::Document::parse_with_options(&text, xml::OPTS) else { return d };
    let rels = pkg.rels(part).unwrap_or_default();
    for anchor in doc
        .descendants()
        .filter(|n| is(*n, "xdr", "twoCellAnchor") || is(*n, "xdr", "oneCellAnchor") || is(*n, "xdr", "absoluteAnchor"))
    {
        let from = child(anchor, "xdr", "from");
        let (col, row, col_off, row_off) = match from {
            Some(f) => (int_child(f, "col"), int_child(f, "row"), int_child(f, "colOff"), int_child(f, "rowOff")),
            None => (0.0, 0.0, 0.0, 0.0),
        };
        let (col, row) = (col as u32, row as u32);
        let anchor_size = || -> Option<(f64, f64)> {
            if let Some(e) = child(anchor, "xdr", "ext") {
                return Some((emu_attr(e, "cx")? / EMU_PER_PX, emu_attr(e, "cy")? / EMU_PER_PX));
            }
            let to = child(anchor, "xdr", "to")?;
            let (tc, tr) = (int_child(to, "col") as u32, int_child(to, "row") as u32);
            let w: f64 =
                (col..tc).map(col_px).sum::<f64>() - col_off / EMU_PER_PX + int_child(to, "colOff") / EMU_PER_PX;
            let h: f64 =
                (row..tr).map(row_px).sum::<f64>() - row_off / EMU_PER_PX + int_child(to, "rowOff") / EMU_PER_PX;
            (w > 0.0 && h > 0.0).then_some((w, h))
        };

        for n in anchor.descendants() {
            if is(n, "xdr", "pic") {
                let Some(embed) = n.descendants().find(|b| is(*b, "a", "blip")).and_then(|b| attr(b, "r", "embed"))
                else {
                    continue;
                };
                let Some(rel) = rels.get(embed).filter(|r| !r.external) else { continue };
                let props = child(n, "xdr", "nvPicPr").and_then(|p| child(p, "xdr", "cNvPr"));
                let ext = child(n, "xdr", "spPr")
                    .and_then(|s| child(s, "a", "xfrm"))
                    .and_then(|x| child(x, "a", "ext"))
                    .and_then(|e| Some((emu_attr(e, "cx")? / EMU_PER_PX, emu_attr(e, "cy")? / EMU_PER_PX)))
                    .filter(|(w, h)| *w > 0.0 && *h > 0.0);
                d.items.push(Item {
                    kind: Kind::Picture(rel.target.clone()),
                    name: props.and_then(|p| p.attribute("name")).unwrap_or("").to_string(),
                    alt: props.and_then(|p| p.attribute("descr")).unwrap_or("").to_string(),
                    row,
                    col,
                    size: ext.or_else(anchor_size),
                });
            } else if is(n, "xdr", "graphicFrame") {
                if n.descendants().any(|c| is(c, "c", "chart")) {
                    let name = child(n, "xdr", "nvGraphicFramePr")
                        .and_then(|p| child(p, "xdr", "cNvPr"))
                        .and_then(|p| p.attribute("name"))
                        .unwrap_or("");
                    d.items.push(Item {
                        kind: Kind::Chart,
                        name: name.to_string(),
                        alt: String::new(),
                        row,
                        col,
                        size: None,
                    });
                } else {
                    d.shapes += 1;
                }
            } else if is(n, "xdr", "sp") || is(n, "xdr", "cxnSp") {
                d.shapes += 1;
            }
        }
    }
    d.items.sort_by_key(|i| (i.row, i.col));
    d
}

fn anchor_name(i: &Item) -> String {
    format!("{}{}", col_letters(i.col), i.row + 1)
}

/// The nodes that show a sheet's drawing after its table, and the picture files they use.
pub fn nodes(d: &Drawing, pkg: &mut Package, out: &mut Out) -> (Vec<Node>, Vec<String>) {
    let mut nodes = Vec::new();
    let mut files = Vec::new();
    for it in &d.items {
        match &it.kind {
            Kind::Picture(part) => {
                nodes.push(Node::el("div").class("xs").text(format!("图片 · 锚定于 {}", anchor_name(it))));
                let body = match out.media(pkg, part) {
                    Ok(m) => {
                        files.push(m.file.clone());
                        let (w, h) = it.size.unwrap_or((m.w as f64, m.h as f64));
                        let (w, h) = if w > 1600.0 { (1600.0, h * 1600.0 / w) } else { (w, h) };
                        node::image(&m.file, w, h, if it.alt.is_empty() { &it.name } else { &it.alt })
                    }
                    Err(why) => node::placeholder(&format!("图片无法预览：{why}")),
                };
                nodes.push(Node::el("div").class("xm").child(body));
            }
            Kind::Chart => {
                nodes.push(Node::el("div").class("xs").text(format!("图表 · 锚定于 {}", anchor_name(it))));
                let label = if it.name.is_empty() { "图表".to_string() } else { format!("图表：{}", it.name) };
                nodes.push(node::placeholder(&format!("{label} (不支持预览)")));
            }
        }
    }
    (nodes, files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_names() {
        let i = Item { kind: Kind::Chart, name: String::new(), alt: String::new(), row: 3, col: 1, size: None };
        assert_eq!(anchor_name(&i), "B4");
        let i = Item { kind: Kind::Chart, name: String::new(), alt: String::new(), row: 0, col: 27, size: None };
        assert_eq!(anchor_name(&i), "AB1");
    }
}
