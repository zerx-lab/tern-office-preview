//! Tables: grid, spans, vertical merges, borders and table-style conditional formatting.

use super::props::{shd_fill, BKey, Bdr, Bdrs, PPr, RPr};
use super::styles::{Cond, TblStyle};
use super::{Ctx, TblLayer};
use crate::css::{self, Side};
use crate::node::Node;
use crate::xml::{attr, child, child_wval, children, elems, is, num, on_off, twips, wval};
use roxmltree::Node as X;
use std::collections::HashMap;
use std::rc::Rc;

/// Weight above which a top-level table is cut into several nodes.
const TABLE_SPLIT: usize = 2500;
/// Cell padding the base style gives (left + right), px.
const CELL_PAD: f64 = 14.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Vm {
    None,
    Restart,
    Continue,
}

struct Cell<'a, 'i> {
    tc: X<'a, 'i>,
    col: usize,
    span: usize,
    vm: Vm,
    rowspan: usize,
    skip: bool,
}

struct Row<'a, 'i> {
    cells: Vec<Cell<'a, 'i>>,
    before: usize,
    after: usize,
    height_px: f64,
}

#[derive(Clone, Copy, Default)]
struct Look {
    first_row: bool,
    last_row: bool,
    first_col: bool,
    last_col: bool,
    no_hband: bool,
    no_vband: bool,
}

fn parse_look(tblpr: Option<X>) -> Look {
    let Some(l) = tblpr.and_then(|p| child(p, "w", "tblLook")) else {
        return Look { first_row: true, first_col: true, no_vband: true, ..Default::default() };
    };
    if let Some(v) = attr(l, "w", "val").and_then(|v| u32::from_str_radix(v.trim(), 16).ok()) {
        if attr(l, "w", "firstRow").is_none() {
            return Look {
                first_row: v & 0x20 != 0,
                last_row: v & 0x40 != 0,
                first_col: v & 0x80 != 0,
                last_col: v & 0x100 != 0,
                no_hband: v & 0x200 != 0,
                no_vband: v & 0x400 != 0,
            };
        }
    }
    let f = |n: &str| attr(l, "w", n).is_some_and(|v| on_off(Some(v)));
    Look {
        first_row: f("firstRow"),
        last_row: f("lastRow"),
        first_col: f("firstColumn"),
        last_col: f("lastColumn"),
        no_hband: f("noHBand"),
        no_vband: f("noVBand"),
    }
}

/// Which table-style conditions apply to a cell, lowest priority first.
#[allow(clippy::too_many_arguments)]
fn cond_names(
    ri: usize,
    col: usize,
    col_end: usize,
    nrows: usize,
    ncols: usize,
    look: Look,
    ts: &TblStyle,
) -> Vec<&'static str> {
    let mut v = Vec::new();
    let is_first_row = look.first_row && ri == 0;
    let is_last_row = look.last_row && ri + 1 == nrows && nrows > 1;
    let is_first_col = look.first_col && col == 0;
    let is_last_col = look.last_col && col_end == ncols && ncols > 1;
    if !look.no_hband && !is_first_row && !is_last_row {
        let idx = ri - usize::from(look.first_row);
        v.push(if (idx / ts.row_band.max(1)).is_multiple_of(2) { "band1Horz" } else { "band2Horz" });
    }
    if !look.no_vband && !is_first_col && !is_last_col {
        let idx = col - usize::from(look.first_col);
        v.push(if (idx / ts.col_band.max(1)).is_multiple_of(2) { "band1Vert" } else { "band2Vert" });
    }
    if is_last_col {
        v.push("lastCol");
    }
    if is_first_col {
        v.push("firstCol");
    }
    if is_last_row {
        v.push("lastRow");
    }
    if is_first_row {
        v.push("firstRow");
    }
    if is_first_row && is_first_col {
        v.push("nwCell");
    }
    if is_first_row && is_last_col {
        v.push("neCell");
    }
    if is_last_row && is_first_col {
        v.push("swCell");
    }
    if is_last_row && is_last_col {
        v.push("seCell");
    }
    v
}

/// The border at `side` of a cell: direct cell border, direct table border, the style's
/// conditions (highest first), the style's table borders.
fn side_border(
    side: Side,
    edge: bool,
    tc: &Bdrs,
    tbl: &Bdrs,
    layers: &[&Cond],
    style: Option<&TblStyle>,
) -> Option<Bdr> {
    let (exact, key) = match (side, edge) {
        (Side::Top, true) => (BKey::Top, BKey::Top),
        (Side::Top, false) => (BKey::Top, BKey::InsideH),
        (Side::Bottom, true) => (BKey::Bottom, BKey::Bottom),
        (Side::Bottom, false) => (BKey::Bottom, BKey::InsideH),
        (Side::Left, true) => (BKey::Left, BKey::Left),
        (Side::Left, false) => (BKey::Left, BKey::InsideV),
        (Side::Right, true) => (BKey::Right, BKey::Right),
        (Side::Right, false) => (BKey::Right, BKey::InsideV),
    };
    tc.get(exact)
        .or_else(|| tbl.get(key))
        .or_else(|| layers.iter().rev().find_map(|c| c.borders.get(exact).or_else(|| c.borders.get(key))))
        .or_else(|| style.and_then(|s| s.borders.get(key)))
}

impl Ctx<'_> {
    pub fn table(&mut self, tbl: X, top: bool) -> Vec<Node> {
        let tblpr = child(tbl, "w", "tblPr");
        let style_id = tblpr.and_then(|p| child_wval(p, "tblStyle"));
        let ts: Option<Rc<TblStyle>> = self.styles.table(style_id);
        let look = parse_look(tblpr);
        let tbl_borders = tblpr
            .and_then(|p| child(p, "w", "tblBorders"))
            .map(|b| Bdrs::parse(b, &self.theme))
            .unwrap_or_default();

        // Rows and cells.
        let mut rows: Vec<Row> = Vec::new();
        for tr in children(tbl, "w", "tr") {
            let trpr = child(tr, "w", "trPr");
            let gv = |name: &str| trpr.and_then(|p| child_wval(p, name)).and_then(|v| num::<usize>(Some(v))).unwrap_or(0);
            let height_px = trpr
                .and_then(|p| child(p, "w", "trHeight"))
                .and_then(|h| twips(attr(h, "w", "val")))
                .map_or(0.0, |t| t / 15.0);
            let mut row = Row { cells: Vec::new(), before: gv("gridBefore"), after: gv("gridAfter"), height_px };
            let mut col = row.before;
            for tc in elems(tr).flat_map(|c| {
                if is(c, "w", "tc") {
                    vec![c]
                } else if is(c, "w", "sdt") {
                    child(c, "w", "sdtContent").map(|s| children(s, "w", "tc").collect()).unwrap_or_default()
                } else {
                    Vec::new()
                }
            }) {
                let tcpr = child(tc, "w", "tcPr");
                let span = tcpr.and_then(|p| child_wval(p, "gridSpan")).and_then(|v| num::<usize>(Some(v))).unwrap_or(1).max(1);
                let vm = match tcpr.and_then(|p| child(p, "w", "vMerge")) {
                    None => Vm::None,
                    Some(v) if wval(v) == Some("restart") => Vm::Restart,
                    Some(_) => Vm::Continue,
                };
                if let Some(h) = tcpr.and_then(|p| child(p, "w", "hMerge")) {
                    if wval(h) != Some("restart") {
                        if let Some(prev) = row.cells.last_mut() {
                            prev.span += span;
                            col += span;
                            continue;
                        }
                    }
                }
                row.cells.push(Cell { tc, col, span, vm, rowspan: 1, skip: false });
                col += span;
            }
            rows.push(row);
        }
        if rows.is_empty() {
            return Vec::new();
        }

        // Grid.
        let mut grid: Vec<f64> = child(tbl, "w", "tblGrid")
            .map(|g| children(g, "w", "gridCol").map(|c| twips(attr(c, "w", "w")).unwrap_or(0.0)).collect())
            .unwrap_or_default();
        let ncols = rows
            .iter()
            .map(|r| r.cells.last().map_or(r.before, |c| c.col + c.span) + r.after)
            .max()
            .unwrap_or(0)
            .max(grid.len())
            .max(1);
        if grid.len() < ncols {
            let mut known: Vec<Option<f64>> = vec![None; ncols];
            for (i, g) in grid.iter().enumerate() {
                if *g > 0.0 {
                    known[i] = Some(*g);
                }
            }
            for r in &rows {
                for c in &r.cells {
                    if c.span == 1 && c.col < ncols && known[c.col].is_none() {
                        if let Some(w) = child(c.tc, "w", "tcPr")
                            .and_then(|p| child(p, "w", "tcW"))
                            .filter(|w| attr(*w, "w", "type").unwrap_or("dxa") == "dxa")
                            .and_then(|w| twips(attr(w, "w", "w")))
                            .filter(|w| *w > 0.0)
                        {
                            known[c.col] = Some(w);
                        }
                    }
                }
            }
            let sum: f64 = known.iter().flatten().sum();
            let n = known.iter().flatten().count();
            let avg = if n > 0 { sum / n as f64 } else { 1800.0 };
            grid = known.into_iter().map(|k| k.unwrap_or(avg)).collect();
        }
        for g in &mut grid {
            if *g <= 0.0 {
                *g = 600.0;
            }
        }

        // Vertical merges → rowspan.
        let nrows = rows.len();
        for ri in 0..nrows {
            for ci in 0..rows[ri].cells.len() {
                if rows[ri].cells[ci].vm != Vm::Restart {
                    continue;
                }
                let (col, span) = (rows[ri].cells[ci].col, rows[ri].cells[ci].span);
                let mut r2 = ri + 1;
                while r2 < nrows {
                    let Some(k) = rows[r2].cells.iter().position(|c| c.col == col && c.vm == Vm::Continue && c.span == span && !c.skip)
                    else {
                        break;
                    };
                    rows[r2].cells[k].skip = true;
                    r2 += 1;
                }
                rows[ri].cells[ci].rowspan = r2 - ri;
            }
        }

        // Column widths in px: scaled to the declared table width / the space available.
        let mut px: Vec<f64> = grid.iter().map(|g| g / 15.0).collect();
        let sum: f64 = px.iter().sum();
        let declared = tblpr.and_then(|p| child(p, "w", "tblW")).and_then(|w| {
            let v = attr(w, "w", "w")?;
            match attr(w, "w", "type") {
                Some("pct") => {
                    let pct = if let Some(p) = v.strip_suffix('%') { p.trim().parse::<f64>().ok()? } else { v.trim().parse::<f64>().ok()? / 50.0 };
                    Some(self.avail * pct / 100.0)
                }
                Some("dxa") | None => twips(Some(v)).map(|t| t / 15.0).filter(|w| *w > 0.0),
                _ => None,
            }
        });
        let mut target = sum;
        if let Some(d) = declared {
            if d > sum * 1.02 || d < sum * 0.98 {
                target = d;
            }
        }
        if target > self.avail {
            target = self.avail;
        }
        if sum > 0.0 && (target - sum).abs() > 1.0 && (target < sum || declared.is_some()) {
            let k = target / sum;
            for w in &mut px {
                *w *= k;
            }
        }

        // Grid occupancy: which (row, cell) covers each grid slot. Continuations of a vertical
        // merge are covered by their restart cell's row span.
        let mut occ: Vec<Vec<Option<(usize, usize)>>> = vec![vec![None; ncols]; nrows];
        for (ri, row) in rows.iter().enumerate() {
            for (ci, c) in row.cells.iter().enumerate() {
                if c.skip {
                    continue;
                }
                for slot_row in occ.iter_mut().take((ri + c.rowspan).min(nrows)).skip(ri) {
                    for slot in slot_row.iter_mut().take((c.col + c.span).min(ncols)).skip(c.col.min(ncols)) {
                        if slot.is_none() {
                            *slot = Some((ri, ci));
                        }
                    }
                }
            }
        }

        // Per-cell formatting: table-style layer, shading and the cell's own four borders.
        struct Fmt {
            layer: Rc<TblLayer>,
            shd: Option<crate::color::Rgb>,
            /// Top, left, bottom, right.
            borders: [Option<css::Border>; 4],
        }
        let mut layer_cache: HashMap<Vec<&'static str>, Rc<TblLayer>> = HashMap::new();
        let default_cond = Cond::default();
        let mut fmts: Vec<Vec<Option<Fmt>>> = Vec::with_capacity(nrows);
        for ri in 0..nrows {
            let mut row_fmt: Vec<Option<Fmt>> = Vec::with_capacity(rows[ri].cells.len());
            for c in &rows[ri].cells {
                if c.skip {
                    row_fmt.push(None);
                    continue;
                }
                let col_end = (c.col + c.span).min(ncols);
                let names = cond_names(ri, c.col, col_end, nrows, ncols, look, ts.as_deref().unwrap_or(&TblStyle::default()));
                let layers: Vec<&Cond> = match &ts {
                    Some(t) => std::iter::once(&t.whole).chain(names.iter().filter_map(|n| t.conds.get(*n))).collect(),
                    None => vec![&default_cond],
                };
                let layer = layer_cache
                    .entry(names)
                    .or_insert_with(|| {
                        let mut ppr = PPr::default();
                        let mut rpr = RPr::default();
                        for l in &layers {
                            ppr.overlay(&l.ppr);
                            rpr.overlay(&l.rpr);
                        }
                        Rc::new(TblLayer { ppr, rpr })
                    })
                    .clone();
                let tcpr = child(c.tc, "w", "tcPr");
                let direct_shd = tcpr.and_then(|p| child(p, "w", "shd")).and_then(|s| shd_fill(s, &self.theme));
                let shd = direct_shd.or_else(|| layers.iter().rev().find_map(|l| l.shd)).flatten();
                let tc_b = tcpr
                    .and_then(|p| child(p, "w", "tcBorders"))
                    .map(|b| Bdrs::parse(b, &self.theme))
                    .unwrap_or_default();
                let mut borders = [None; 4];
                for (k, (side, edge)) in [
                    (Side::Top, ri == 0),
                    (Side::Left, c.col == 0),
                    (Side::Bottom, ri + c.rowspan >= nrows),
                    (Side::Right, col_end >= ncols),
                ]
                .into_iter()
                .enumerate()
                {
                    borders[k] = side_border(side, edge, &tc_b, &tbl_borders, &layers, ts.as_deref()).and_then(|b| b.border());
                }
                row_fmt.push(Some(Fmt { layer, shd, borders }));
            }
            fmts.push(row_fmt);
        }

        // Cells in row-major order, gaps (gridBefore / gridAfter / short rows) as empty placeholders.
        let span_px = |from: usize, to: usize| -> f64 { px[from.min(ncols - 1)..to.max(from + 1).min(ncols)].iter().sum() };
        let mut row_nodes: Vec<Vec<Node>> = Vec::with_capacity(nrows);
        let mut breakable: Vec<bool> = Vec::with_capacity(nrows);
        let mut open_until = 0usize;
        let mut done: Vec<Vec<bool>> = rows.iter().map(|r| vec![false; r.cells.len()]).collect();
        for ri in 0..nrows {
            let mut cells_out: Vec<Node> = Vec::new();
            let row_h = rows[ri].height_px;
            let mut col = 0;
            while col < ncols {
                match occ[ri][col] {
                    None => {
                        let mut end = col + 1;
                        while end < ncols && occ[ri][end].is_none() {
                            end += 1;
                        }
                        let mut cls = css::width(span_px(col, end)).unwrap_or_default();
                        if let Some(sp) = css::col_span(end - col) {
                            cls.push(' ');
                            cls.push_str(&sp);
                        }
                        cells_out.push(Node::el("div").class(cls.trim()));
                        col = end;
                    }
                    Some((r, ci)) if r != ri || done[r][ci] || rows[r].cells[ci].col != col => col += 1,
                    Some((_, ci)) => {
                        done[ri][ci] = true;
                        let (tc, span, rowspan) = {
                            let c = &rows[ri].cells[ci];
                            (c.tc, c.span, c.rowspan)
                        };
                        let col_end = (col + span).min(ncols);
                        open_until = open_until.max(ri + rowspan - 1);
                        let f = fmts[ri][ci].as_ref().expect("formatted cell");
                        let tcpr = child(tc, "w", "tcPr");
                        let w_px = span_px(col, col_end);
                        let mut cls = css::width(w_px).unwrap_or_default();
                        let mut add = |c: Option<String>| {
                            if let Some(c) = c {
                                cls.push(' ');
                                cls.push_str(&c);
                            }
                        };
                        add(css::col_span(col_end - col));
                        add(css::row_span(rowspan));
                        add(f.shd.map(css::bg));
                        // Each cell draws its right and bottom edges (its own, else the neighbour's
                        // facing edge); left / top only where no cell on that side draws one.
                        let nb = |at: Option<(usize, usize)>, k: usize| at.and_then(|(r, c)| fmts[r][c].as_ref()).and_then(|n| n.borders[k]);
                        let right_at = if col_end < ncols { occ[ri][col_end] } else { None };
                        let below_at = if ri + rowspan < nrows { occ[ri + rowspan][col] } else { None };
                        let right = f.borders[3].or_else(|| nb(right_at, 1));
                        let bottom = f.borders[2].or_else(|| nb(below_at, 0));
                        let left = if col == 0 || occ[ri][col - 1].is_none() { f.borders[1] } else { None };
                        let top = if ri == 0 || occ[ri - 1][col].is_none() { f.borders[0] } else { None };
                        let mut bcls = String::new();
                        for (side, b) in [(Side::Top, top), (Side::Left, left), (Side::Bottom, bottom), (Side::Right, right)] {
                            if let Some(b) = b {
                                css::border(&mut bcls, side, b);
                            }
                        }
                        add(Some(bcls));
                        match tcpr.and_then(|p| child_wval(p, "vAlign")) {
                            Some("center") => add(Some("vm".into())),
                            Some("bottom") => add(Some("vb".into())),
                            _ => {}
                        }
                        if row_h >= 24.0 && rowspan == 1 {
                            add(css::height(row_h));
                        }
                        let mut cell = Node::el("div").class(&cls.split_whitespace().collect::<Vec<_>>().join(" "));

                        // Content.
                        let saved_tbl = self.tbl.replace(f.layer.clone());
                        let saved_avail = self.avail;
                        let saved_outline = std::mem::replace(&mut self.outline_on, false);
                        self.avail = (w_px - CELL_PAD).max(40.0);
                        let mut kids = Vec::new();
                        self.blocks(tc, &mut kids);
                        cell.children = kids;
                        self.tbl = saved_tbl;
                        self.avail = saved_avail;
                        self.outline_on = saved_outline;
                        cells_out.push(cell);
                        col = col_end;
                    }
                }
            }
            breakable.push(open_until <= ri);
            row_nodes.push(cells_out);
        }

        // The grid element(s).
        let mut tcls = css::grid_cols(ncols);
        tcls.insert_str(0, "dg ");
        match tblpr.and_then(|p| child_wval(p, "jc")) {
            Some("center") => tcls.push_str(" tjc"),
            Some("right") | Some("end") => tcls.push_str(" tjr"),
            _ => {}
        }
        let ind_px = tblpr
            .and_then(|p| child(p, "w", "tblInd"))
            .and_then(|i| twips(attr(i, "w", "w")))
            .map_or(0.0, |t| t / 15.0);
        if ind_px >= 8.0 {
            if let Some(c) = css::pad_left(ind_px) {
                tcls.push(' ');
                tcls.push_str(&c);
            }
        }
        let mk = |rows: Vec<Vec<Node>>| -> Node {
            let mut g = Node::el("div").class(&tcls);
            g.children = rows.into_iter().flatten().collect();
            g
        };
        let weight = |r: &Vec<Node>| r.iter().map(Node::weight).sum::<usize>();
        let total: usize = row_nodes.iter().map(weight).sum();
        if !top || total <= TABLE_SPLIT {
            return vec![mk(row_nodes)];
        }
        let mut out = Vec::new();
        let mut cur: Vec<Vec<Node>> = Vec::new();
        let mut w = 0usize;
        for (i, r) in row_nodes.into_iter().enumerate() {
            w += weight(&r);
            cur.push(r);
            if w >= TABLE_SPLIT && breakable[i] {
                out.push(mk(std::mem::take(&mut cur)));
                w = 0;
            }
        }
        if !cur.is_empty() {
            out.push(mk(cur));
        }
        out
    }
}
