//! Turns a parsed worksheet into grid parts: layout (visible rows / columns, part split), merged
//! cells, border normalization, text overflow, frozen rows, and the JSON writer. The sheet is a
//! CSS grid of `div`s (Tern's table layout ignores colspan/rowspan): one column per visible
//! column plus the row-number column, spans as `gs{n}` / `gr{n}` classes.
//!
//! Parts are serialized straight to JSON (the node shape `{k, p, c}`): a sheet can have millions of
//! cells and building a `Node` tree for each would dominate the render time.

use super::numfmt::Format;
use super::sheet::{col_letters, Cell, Merge, Sheet, Val, MAX_COLS, MAX_ROWS, NO_XF};
use super::strings::{Run, Strings};
use super::styles::{HAlign, Styles, Xf, BOTTOM, LEFT, RIGHT, TOP};
use crate::color::Rgb;
use crate::css::{self, Side};
use std::collections::HashMap;
use std::fmt::Write;
use std::ops::Range;

/// Most cells (visible rows × columns) one sheet renders; more are cut with a note.
const CELL_BUDGET: usize = 3_000_000;
/// Most frozen rows repeated at the top of every part.
const MAX_FROZEN: usize = 10;
/// Extra px added to `width × 7` for a column. Excel's stored `width` already includes the cell
/// padding, so none is added.
const COL_PX_EXTRA: f64 = 0.0;
/// Horizontal padding (px) inside a cell, for the overflow estimate.
const CELL_PAD: f64 = 6.0;
const NONE: u32 = u32::MAX;
/// About how many nodes one part holds (Tern diffs and ships every node of the shown part).
/// Plain sheets stay under it at `rpp` rows; rows heavy with rich text end a part early.
const PART_NODES: usize = 7000;

/// Everything a part needs from the workbook.
pub struct Ctx<'a> {
    pub sheet: &'a Sheet,
    pub styles: &'a Styles,
    pub strings: &'a Strings,
    pub date1904: bool,
    date_fmt: Format,
    datetime_fmt: Format,
}

impl<'a> Ctx<'a> {
    pub fn new(sheet: &'a Sheet, styles: &'a Styles, strings: &'a Strings, date1904: bool) -> Ctx<'a> {
        Ctx {
            sheet,
            styles,
            strings,
            date1904,
            date_fmt: crate::xlsx::numfmt::parse("yyyy/m/d"),
            datetime_fmt: crate::xlsx::numfmt::parse("yyyy/m/d h:mm:ss"),
        }
    }
}

/// A column width as the 4px-step `w{n}` class renders it, so spans sum to the same tracks.
fn quant(px: f64) -> f64 {
    (px / 4.0).round().clamp(1.0, 600.0) * 4.0
}

/// Excel column width (character units, padding included) → px.
pub fn width_px(w: f64) -> f64 {
    (w * 7.0 + COL_PX_EXTRA).round()
}

fn default_width_px(sh: &Sheet) -> f64 {
    if let Some(w) = sh.default_col_w {
        return width_px(w).max(1.0);
    }
    let base = sh.base_col_w.unwrap_or(8.0);
    ((base * 7.0 + 5.0) / 8.0).ceil() * 8.0
}

/// Which rows and columns show, and how the rows split into parts.
pub struct Plan {
    /// Visible columns (0-based sheet indexes).
    pub vcols: Vec<u32>,
    vcol_of: Vec<u32>,
    pub col_px: Vec<f64>,
    col_xf: Vec<u16>,
    /// Width of every column 0..ncols (hidden = 0), for sizing drawings.
    pub all_px: Vec<f64>,
    pub default_px: f64,
    /// Visible row numbers, in order.
    pub vrows: Vec<u32>,
    row_meta: Vec<u32>,
    row_h: Vec<u16>,
    /// Frozen rows repeated on every part after the first.
    pub frozen: usize,
    pub parts: Vec<Range<usize>>,
    merges: Vec<Merge>,
    merge_pmax: Vec<u32>,
    links: HashMap<u64, u32>,
    pub notes: Vec<String>,
    /// Rows / columns holding data (including those cut off).
    pub used_rows: u32,
    pub used_cols: u32,
}

fn link_key(row: u32, col: u32) -> u64 {
    ((row as u64) << 20) | col as u64
}

impl Plan {
    pub fn build(ctx: &Ctx) -> Plan {
        let sh = ctx.sheet;
        // Extent of what is worth drawing: values, fills or borders.
        let mut last_row = 0u32;
        let mut max_col = 0usize;
        for row in &sh.rows {
            if let Some(c) =
                sh.cells_of(row).iter().rev().find(|c| c.val != Val::Empty || ctx.styles.xf(c.xf as usize).visible())
            {
                max_col = max_col.max(c.col as usize + 1);
                last_row = last_row.max(row.r);
            }
        }
        for m in &sh.merges {
            if m.r0 <= last_row {
                max_col = max_col.max(m.c1 as usize + 1);
            }
        }
        let ncols = max_col.min(MAX_COLS as usize);

        let default_px = default_width_px(sh);
        let mut all_px = vec![default_px; ncols];
        let mut hidden = vec![false; ncols];
        let mut xf = vec![NO_XF; ncols];
        for d in &sh.cols {
            for c in d.min as usize..=(d.max as usize).min(ncols.saturating_sub(1)) {
                if c >= ncols {
                    break;
                }
                if let Some(w) = d.width {
                    all_px[c] = width_px(w);
                }
                hidden[c] = d.hidden || (d.width == Some(0.0));
                xf[c] = d.xf;
            }
        }
        let mut vcols = Vec::new();
        let mut vcol_of = vec![NONE; ncols];
        let mut col_px = Vec::new();
        let mut col_xf = Vec::new();
        for c in 0..ncols {
            if hidden[c] || all_px[c] < 1.0 {
                all_px[c] = 0.0;
                continue;
            }
            vcol_of[c] = vcols.len() as u32;
            vcols.push(c as u32);
            col_px.push(all_px[c]);
            col_xf.push(xf[c]);
        }

        // Visible rows.
        let nv = vcols.len().max(1);
        let max_rows = (CELL_BUDGET / nv).min(MAX_ROWS as usize);
        let mut vrows = Vec::new();
        let mut row_meta = Vec::new();
        let mut row_h = Vec::new();
        let mut p = 0usize;
        let mut cut_at = None;
        for r in 1..=last_row {
            while p < sh.rows.len() && sh.rows[p].r < r {
                p += 1;
            }
            let meta = if p < sh.rows.len() && sh.rows[p].r == r { Some(sh.rows[p]) } else { None };
            if meta.is_some_and(|m| m.hidden) {
                continue;
            }
            if vrows.len() >= max_rows {
                cut_at = Some(r);
                break;
            }
            let pt = match meta {
                Some(m) if m.ht > 0.0 => m.ht as f64,
                _ => sh.default_row_ht.unwrap_or(0.0),
            };
            let n = if pt > 0.0 { ((pt * 4.0 / 3.0) / 2.0).round().clamp(1.0, 300.0) as u16 } else { 0 };
            vrows.push(r);
            row_meta.push(if meta.is_some() { p as u32 } else { NONE });
            row_h.push(if n == 10 { 0 } else { n });
        }

        let mut notes = Vec::new();
        let used_rows = if sh.over_rows > 0 { sh.over_rows } else { last_row };
        if cut_at.is_some() || sh.over_rows > 0 {
            let shown = vrows.last().copied().unwrap_or(0);
            notes.push(format!("仅显示前 {shown} 行（共 {used_rows} 行）"));
        }
        let used_cols = (max_col as u32).max(sh.over_cols);
        if sh.over_cols > 0 {
            notes.push(format!("仅显示前 {} 列（共 {} 列）", MAX_COLS, sh.over_cols));
        }

        // Parts.
        let rpp = (5000 / nv).clamp(40, 500);
        let mut frozen = vrows.iter().take_while(|r| **r <= sh.frozen_rows).count().min(MAX_FROZEN);
        if frozen >= vrows.len() || frozen * 2 > rpp {
            frozen = 0;
        }
        // A part holds at most `rpp` rows and about PART_NODES nodes: each row is a header cell
        // plus one div per column, and every rich-text cell adds a span and one per run.
        let row_weight = |i: usize| -> usize {
            let mut w = 1 + nv;
            let m = row_meta[i];
            if m != NONE {
                for c in sh.cells_of(&sh.rows[m as usize]) {
                    if let Val::Rich(ri) = c.val {
                        w += 1 + sh.rich.get(ri as usize).map_or(0, |runs| runs.len());
                    }
                }
            }
            w
        };
        let mut parts = Vec::new();
        let mut s = 0;
        while s < vrows.len() {
            let mut e = s;
            let mut w = frozen * (2 + nv);
            while e < vrows.len() && e - s < rpp {
                let rw = row_weight(e);
                if e > s && w + rw > PART_NODES {
                    break;
                }
                w += rw;
                e += 1;
            }
            parts.push(s..e);
            s = e;
        }

        let mut merges: Vec<Merge> = sh.merges.iter().filter(|m| m.r0 <= last_row).copied().collect();
        merges.sort_by_key(|m| (m.r0, m.c0));
        let mut merge_pmax = Vec::with_capacity(merges.len());
        let mut mx = 0;
        for m in &merges {
            mx = mx.max(m.r1);
            merge_pmax.push(mx);
        }

        let mut links = HashMap::new();
        let mut budget = 200_000usize;
        for (i, l) in sh.links.iter().enumerate() {
            let r = l.range;
            let area = (r.r1 - r.r0 + 1) as usize * (r.c1 - r.c0 + 1) as usize;
            if area > budget {
                continue;
            }
            budget -= area;
            for row in r.r0..=r.r1 {
                for col in r.c0..=r.c1 {
                    links.entry(link_key(row, col)).or_insert(i as u32);
                }
            }
        }

        Plan {
            vcols,
            vcol_of,
            col_px,
            col_xf,
            all_px,
            default_px,
            vrows,
            row_meta,
            row_h,
            frozen,
            parts,
            merges,
            merge_pmax,
            links,
            notes,
            used_rows,
            used_cols,
        }
    }

    /// `1–200`: the Excel row numbers a part covers.
    pub fn label(&self, part: usize) -> String {
        let Some(r) = self.parts.get(part) else { return String::new() };
        let (a, b) = (self.vrows[r.start], self.vrows[r.end - 1]);
        if a == b {
            a.to_string()
        } else {
            format!("{a}–{b}")
        }
    }

    /// The JSON of part `idx`: the root `div.opv.opx` with notes, the cell grid, and `extra`
    /// (already serialized nodes, comma separated) after the grid.
    pub fn render_part(&self, ctx: &Ctx, idx: usize, notes: &[String], extra: &str, no_grid: bool) -> String {
        let mut o = String::with_capacity(64 * 1024);
        o.push_str(r#"{"k":"el","p":{"tag":"div","class":"opv opx"},"c":["#);
        let mut first = true;
        let mut sep = |o: &mut String| {
            if !first {
                o.push(',');
            }
            first = false;
        };
        for n in notes {
            sep(&mut o);
            o.push_str(r#"{"k":"el","p":{"tag":"div","class":"xmsg","text":""#);
            push_text(&mut o, n, false);
            o.push_str(r#""}}"#);
        }
        if idx < self.parts.len() {
            sep(&mut o);
            self.write_table(ctx, idx, no_grid, &mut o);
        }
        if !extra.is_empty() {
            sep(&mut o);
            o.push_str(extra);
        }
        o.push_str("]}");
        o
    }

    // (see `quant`)
    fn write_table(&self, ctx: &Ctx, idx: usize, no_grid: bool, o: &mut String) {
        let sh = ctx.sheet;
        let st = ctx.styles;
        let nv = self.vcols.len();
        let range = self.parts[idx].clone();
        // Display rows: (index into vrows, segment).
        let mut disp: Vec<(usize, u8)> = Vec::with_capacity(range.len() + self.frozen);
        if idx > 0 && self.frozen > 0 {
            disp.extend((0..self.frozen).map(|vi| (vi, 0u8)));
            disp.extend(range.clone().map(|vi| (vi, 1u8)));
        } else {
            disp.extend(range.clone().map(|vi| (vi, 0u8)));
        }
        let nd = disp.len();
        let tail_vi = range.end;
        let has_tail = tail_vi < self.vrows.len();

        // Grid of cell formats (+ the row below) and where each cell lives in the row's cells.
        let mut grid = vec![NO_XF; (nd + 1) * nv];
        let mut cellref = vec![NONE; nd * nv];
        for k in 0..=nd {
            let vi = if k < nd {
                disp[k].0
            } else if has_tail {
                tail_vi
            } else {
                break;
            };
            let meta = self.row_meta[vi];
            let (row_xf, cells): (u16, &[Cell]) = if meta == NONE {
                (NO_XF, &[])
            } else {
                let r = &sh.rows[meta as usize];
                (r.xf, sh.cells_of(r))
            };
            for ci in 0..nv {
                grid[k * nv + ci] = if row_xf != NO_XF {
                    row_xf
                } else if self.col_xf[ci] != NO_XF {
                    self.col_xf[ci]
                } else {
                    0
                };
            }
            for (n, c) in cells.iter().enumerate() {
                let ci = self.vcol_of.get(c.col as usize).copied().unwrap_or(NONE);
                if ci != NONE {
                    grid[k * nv + ci as usize] = c.xf;
                    if k < nd {
                        cellref[k * nv + ci as usize] = n as u32;
                    }
                }
            }
        }
        let xf_at = |k: usize, ci: usize| -> &Xf { st.xf(grid[k * nv + ci] as usize) };
        let below_exists = |k: usize| k < nd || (k == nd && has_tail);

        // Merged cells, clipped to this part's segments.
        let mut rs = vec![1u16; nd * nv];
        let mut cs = vec![1u16; nd * nv];
        let mut real = vec![0u8; nd * nv];
        let mut ovr: HashMap<usize, (u32, u32)> = HashMap::new();
        let segs: Vec<Range<usize>> =
            if idx > 0 && self.frozen > 0 { vec![0..self.frozen, self.frozen..nd] } else { vec![0..nd] };
        for seg in segs {
            if seg.is_empty() {
                continue;
            }
            let sl = &disp[seg.clone()];
            let (min_r, max_r) = (self.vrows[sl[0].0], self.vrows[sl[sl.len() - 1].0]);
            let a = self.merge_pmax.partition_point(|&x| x < min_r);
            let b = self.merges.partition_point(|m| m.r0 <= max_r);
            for m in self.merges.get(a..b.max(a)).unwrap_or(&[]) {
                if m.r1 < min_r {
                    continue;
                }
                let k0 = sl.partition_point(|d| self.vrows[d.0] < m.r0);
                let k1 = sl.partition_point(|d| self.vrows[d.0] <= m.r1);
                let c0 = self.vcols.partition_point(|c| *c < m.c0);
                let c1 = self.vcols.partition_point(|c| *c <= m.c1);
                if k0 >= k1 || c0 >= c1 {
                    continue;
                }
                let (k0, k1) = (k0 + seg.start, k1 + seg.start);
                let tl = k0 * nv + c0;
                if rs[tl] != 1 || cs[tl] != 1 || (k0..k1).any(|k| (c0..c1).any(|c| rs[k * nv + c] == 0)) {
                    continue;
                }
                for k in k0..k1 {
                    for c in c0..c1 {
                        rs[k * nv + c] = 0;
                    }
                }
                rs[tl] = (k1 - k0) as u16;
                cs[tl] = (c1 - c0) as u16;
                real[tl] = if m.r0 < min_r {
                    1
                } else if self.vrows[disp[k0].0] == m.r0 && self.vcols[c0] == m.c0 {
                    0
                } else {
                    ovr.insert(tl, (m.r0, m.c0));
                    2
                };
            }
        }

        // Header row: the corner, then one letter per visible column.
        o.push_str(r#"{"k":"el","p":{"tag":"div","class":"xg "#);
        o.push_str(&css::grid_cols(nv + 1));
        if !st.table_class.is_empty() {
            o.push(' ');
            o.push_str(&st.table_class);
        }
        if no_grid {
            o.push_str(" opxng");
        }
        o.push_str(r#""},"c":["#);
        o.push_str(r#"{"k":"el","p":{"tag":"div","class":"xc","key":"x"}}"#);
        for (ci, &c) in self.vcols.iter().enumerate() {
            o.push_str(r#",{"k":"el","p":{"tag":"div","class":"xh"#);
            if let Some(w) = css::width(self.col_px[ci]) {
                let _ = write!(o, " {w}");
            }
            let _ = write!(o, r#"","key":"h{}","actions":{{"click":"c"}},"text":"{}"}}}}"#, c + 1, col_letters(c));
        }

        let mut buf = String::new();
        let mut cls = String::new();
        for k in 0..nd {
            let vi = disp[k].0;
            let row_no = self.vrows[vi];
            let meta = self.row_meta[vi];
            let hn = self.row_h[vi];
            o.push_str(r#",{"k":"el","p":{"tag":"div","class":"xr"#);
            if hn > 0 {
                let _ = write!(o, " h{hn}");
            }
            let _ = write!(o, r#"","key":"r{row_no}","actions":{{"click":"c"}},"text":"{row_no}"}}}}"#);
            let cells: &[Cell] = if meta == NONE { &[] } else { sh.cells_of(&sh.rows[meta as usize]) };
            let is_fz = self.frozen > 0 && vi + 1 == self.frozen;
            let mut ci = 0;
            while ci < nv {
                let g = k * nv + ci;
                if rs[g] == 0 {
                    ci += 1;
                    continue;
                }
                let mut rspan = rs[g] as usize;
                let mut cspan = cs[g] as usize;
                let merged = rspan > 1 || cspan > 1;
                let cell = if cellref[g] == NONE { None } else { Some(&cells[cellref[g] as usize]) };
                let xf = xf_at(k, ci);
                let val = match real[g] {
                    0 => cell.map_or(Val::Empty, |c| c.val),
                    1 => Val::Empty,
                    _ => ovr.get(&g).and_then(|&(r, c)| find_cell(sh, r, c)).map_or(Val::Empty, |c| c.val),
                };
                buf.clear();
                let (kind, color, runs) = render_val(ctx, val, xf, &mut buf);

                // Text spills into the empty cells to its right.
                if !merged
                    && kind == Vk::Text
                    && !xf.wrap
                    && matches!(xf.halign, HAlign::General | HAlign::Left)
                    && xf.borders[RIGHT].is_none()
                {
                    let need = text_width(&buf, xf.font_px);
                    let mut have = self.col_px[ci] - CELL_PAD;
                    let mut j = 1;
                    while need > have && ci + j < nv {
                        let g2 = g + j;
                        let blank = rs[g2] == 1
                            && cs[g2] == 1
                            && (cellref[g2] == NONE || cells[cellref[g2] as usize].val == Val::Empty)
                            && !xf_at(k, ci + j).visible();
                        if !blank {
                            break;
                        }
                        have += self.col_px[ci + j];
                        j += 1;
                    }
                    if j > 1 {
                        cspan = j;
                        for i in 1..j {
                            rs[g + i] = 0;
                        }
                    }
                }
                rspan = rspan.max(1);

                // Text still wider than its cell is clipped there, as Excel does: pin the cell to
                // its columns' width (so the column can't grow to fit it) and keep the whole text
                // as the tooltip.
                let mut clip_px = 0.0;
                if !xf.wrap && kind == Vk::Text {
                    let need = match runs {
                        Some(rs) => rs.iter().map(|r| text_width(&r.text, xf.font_px)).sum(),
                        None => text_width(&buf, xf.font_px),
                    };
                    let span_px: f64 = self.col_px[ci..(ci + cspan).min(nv)].iter().sum();
                    if need > span_px - CELL_PAD {
                        clip_px = span_px;
                    }
                }

                // Classes.
                cls.clear();
                cls.push_str(&xf.class);
                let add = add_class;
                match (color, &xf.fg) {
                    (Some(c), _) => add(&mut cls, &css::fg(c)),
                    (None, Some(f)) => add(&mut cls, f),
                    _ => {}
                }
                let eff = match xf.halign {
                    HAlign::General => match kind {
                        Vk::Num => HAlign::Right,
                        Vk::Bool | Vk::Err => HAlign::Center,
                        _ => HAlign::Left,
                    },
                    a => a,
                };
                if xf.halign == HAlign::General {
                    match kind {
                        Vk::Num => add(&mut cls, "ar"),
                        Vk::Bool | Vk::Err => add(&mut cls, "ac"),
                        _ => {}
                    }
                }
                match eff {
                    HAlign::Left => {
                        if let Some(i) = &xf.ind_left {
                            add(&mut cls, i);
                        }
                    }
                    HAlign::Right => {
                        if let Some(i) = &xf.ind_right {
                            add(&mut cls, i);
                        }
                    }
                    _ => {}
                }
                // Borders: only this cell's right and bottom edges (its own, else the neighbour's
                // facing one), plus left/top at the table's first column/row.
                let kr = k + rspan - 1;
                let cr = ci + cspan - 1;
                let edge_r = xf_at(k, cr);
                let right =
                    edge_r.borders[RIGHT].or_else(|| if cr + 1 < nv { xf_at(k, cr + 1).borders[LEFT] } else { None });
                match right {
                    Some(b) => css::border(&mut cls, Side::Right, b),
                    None if !no_grid && (edge_r.fill || (cr + 1 < nv && xf_at(k, cr + 1).fill)) => {
                        add(&mut cls, &css::no_border(Side::Right))
                    }
                    None => {}
                }
                let edge_b = xf_at(kr, ci);
                let below = below_exists(kr + 1);
                let bottom =
                    edge_b.borders[BOTTOM].or_else(|| if below { xf_at(kr + 1, ci).borders[TOP] } else { None });
                match bottom {
                    Some(b) => css::border(&mut cls, Side::Bottom, b),
                    None if !no_grid && (edge_b.fill || (below && xf_at(kr + 1, ci).fill)) => {
                        add(&mut cls, &css::no_border(Side::Bottom))
                    }
                    None => {}
                }
                if ci == 0 {
                    if let Some(b) = xf.borders[LEFT] {
                        css::border(&mut cls, Side::Left, b);
                    }
                }
                if k == 0 {
                    if let Some(b) = xf.borders[TOP] {
                        css::border(&mut cls, Side::Top, b);
                    }
                }
                // Every cell is pinned to the width of the columns it spans (the grid tracks are
                // max-content, so an unpinned cell would stretch its column).
                let span_q: f64 = (ci..ci + cspan).map(|i| quant(self.col_px[i])).sum();
                if let Some(w) = css::width(span_q) {
                    add(&mut cls, &w);
                }
                if let Some(c) = css::col_span(cspan) {
                    add(&mut cls, &c);
                }
                match css::row_span(rspan) {
                    Some(r) => add(&mut cls, &r),
                    None if hn > 0 => {
                        let _ = write!(cls, " h{hn}");
                    }
                    None => {}
                }
                if is_fz {
                    add(&mut cls, "fz");
                }
                let link = if real[g] != 1 { self.links.get(&link_key(row_no, self.vcols[ci])) } else { None };
                if link.is_some() {
                    add(&mut cls, "lnk");
                }

                // The cell.
                o.push_str(r#",{"k":"el","p":{"tag":"div","class":""#);
                o.push_str(cls.trim_start());
                let _ = write!(o, r#"","key":"{}:{}","actions":{{"click":"c"}}"#, row_no, self.vcols[ci] + 1);
                if !buf.is_empty() {
                    o.push_str(r#","text":""#);
                    push_text(o, &buf, xf.wrap);
                    o.push('"');
                }
                if clip_px > 0.0 {
                    o.push_str(r#","title":""#);
                    match runs {
                        Some(rs) => {
                            for r in rs {
                                push_text(o, &r.text, true);
                            }
                        }
                        None => push_text(o, &buf, true),
                    }
                    o.push('"');
                }
                if let Some(&li) = link {
                    o.push_str(r#","href":""#);
                    push_text(o, &sh.links[li as usize].url, true);
                    o.push('"');
                }
                o.push('}');
                if let Some(runs) = runs {
                    // The cell is a flex column: one span keeps all runs on one line.
                    o.push_str(r#","c":[{"k":"el","p":{"tag":"span"},"c":["#);
                    let mut firstrun = true;
                    for r in runs {
                        if !firstrun {
                            o.push(',');
                        }
                        firstrun = false;
                        o.push_str(r#"{"k":"el","p":{"tag":"span""#);
                        if !r.class.is_empty() {
                            let _ = write!(o, r#","class":"{}""#, r.class);
                        }
                        o.push_str(r#","text":""#);
                        push_text(o, &r.text, xf.wrap);
                        o.push_str(r#""}}"#);
                    }
                    o.push_str("]}]");
                }
                o.push('}');
                ci += cspan;
            }
        }
        o.push_str("]}");
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Vk {
    Empty,
    Text,
    Num,
    Bool,
    Err,
}

/// Formats a cell value into `buf`; rich text comes back as runs instead.
fn render_val<'a>(ctx: &'a Ctx, val: Val, xf: &Xf, buf: &mut String) -> (Vk, Option<Rgb>, Option<&'a [Run]>) {
    let fmt = &ctx.styles.formats[xf.fmt.min(ctx.styles.formats.len() - 1)];
    let text = |s: &str, buf: &mut String| -> (Vk, Option<Rgb>, Option<&'a [Run]>) {
        let color = if !fmt.is_general() && fmt.has_text_section() {
            fmt.text(s, buf)
        } else {
            buf.push_str(s);
            None
        };
        (if buf.is_empty() { Vk::Empty } else { Vk::Text }, color, None)
    };
    match val {
        Val::Empty => (Vk::Empty, None, None),
        Val::Num(v) => {
            let c = fmt.number(v, ctx.date1904, buf);
            (Vk::Num, c, None)
        }
        Val::Date(v) => {
            let f = if fmt.is_date() {
                fmt
            } else if v.fract() != 0.0 {
                &ctx.datetime_fmt
            } else {
                &ctx.date_fmt
            };
            let c = f.number(v, ctx.date1904, buf);
            (Vk::Num, c, None)
        }
        Val::Shared(i) => match ctx.strings.rich(i) {
            Some(runs) => (Vk::Text, None, Some(runs)),
            None => text(ctx.strings.get(i).unwrap_or(""), buf),
        },
        Val::Text(o, l) => text(ctx.sheet.str_at(o, l), buf),
        Val::Rich(i) => match ctx.sheet.rich.get(i as usize) {
            Some(runs) => (Vk::Text, None, Some(runs.as_slice())),
            None => (Vk::Empty, None, None),
        },
        Val::Bool(b) => {
            buf.push_str(if b { "TRUE" } else { "FALSE" });
            (Vk::Bool, None, None)
        }
        Val::Err(o, l) => {
            buf.push_str(ctx.sheet.str_at(o, l));
            (Vk::Err, None, None)
        }
    }
}

fn add_class(cls: &mut String, c: &str) {
    if !cls.is_empty() {
        cls.push(' ');
    }
    cls.push_str(c);
}

/// The cell at `row` (1-based) and `col` (0-based), wherever it sits in the sheet.
fn find_cell(sh: &Sheet, row: u32, col: u32) -> Option<&Cell> {
    let r = sh.rows.binary_search_by_key(&row, |r| r.r).ok()?;
    let cells = sh.cells_of(&sh.rows[r]);
    let i = cells.binary_search_by_key(&(col as u16), |c| c.col).ok()?;
    Some(&cells[i])
}

fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE6F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFD)
}

/// Estimated width in px: CJK ≈ 2 × and the rest ≈ 1 × of `0.55 × font size`.
fn text_width(s: &str, font_px: f64) -> f64 {
    let units: f64 = s
        .chars()
        .map(|c| {
            if is_wide(c) {
                2.0
            } else if c == '\n' {
                0.0
            } else {
                1.0
            }
        })
        .sum();
    units * font_px * 0.55
}

/// Appends `s` JSON-escaped (without quotes), dropping control characters and, unless `keep_nl`,
/// line breaks.
fn push_text(out: &mut String, s: &str, keep_nl: bool) {
    let mut start = 0;
    for (i, c) in s.char_indices() {
        let rep = match c {
            '"' => "\\\"",
            '\\' => "\\\\",
            '\n' => {
                if keep_nl {
                    "\\n"
                } else {
                    ""
                }
            }
            '\t' => "\\t",
            '\r' => "",
            '\u{2028}' => "\\u2028",
            '\u{2029}' => "\\u2029",
            c if (c as u32) < 0x20 => "",
            _ => continue,
        };
        out.push_str(&s[start..i]);
        out.push_str(rep);
        start = i + c.len_utf8();
    }
    out.push_str(&s[start..]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Theme;
    use crate::pkg::Rels;
    use crate::xlsx::sheet::{self, MAX_COLS};
    use crate::xlsx::styles::ColorCtx;
    use serde_json::Value;

    const STYLES: &str = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<numFmts count="1"><numFmt numFmtId="164" formatCode="[Red]0.0"/></numFmts>
<fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><color rgb="FF0000FF"/></font></fonts>
<fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/></patternFill></fill></fills>
<borders count="3"><border><left/><right/><top/><bottom/></border>
<border><left style="thin"/><right style="thin"/><top style="thin"/><bottom style="thin"/></border>
<border><left/><right style="medium"><color rgb="FFFF0000"/></right><top/><bottom/></border></borders>
<cellXfs count="8">
<xf numFmtId="0" fontId="0" fillId="0" borderId="0"/>
<xf numFmtId="0" fontId="1" fillId="0" borderId="0"/>
<xf numFmtId="0" fontId="0" fillId="2" borderId="0"/>
<xf numFmtId="0" fontId="0" fillId="0" borderId="1"/>
<xf numFmtId="164" fontId="1" fillId="0" borderId="0"/>
<xf numFmtId="14" fontId="0" fillId="0" borderId="0"/>
<xf numFmtId="0" fontId="0" fillId="0" borderId="2"/>
<xf numFmtId="0" fontId="0" fillId="0" borderId="0" applyAlignment="1"><alignment wrapText="1" horizontal="right"/></xf>
</cellXfs></styleSheet>"#;

    struct Fx {
        sheet: Sheet,
        styles: Styles,
        strings: Strings,
    }

    fn fx(sheet_xml: &str, sst: &str) -> Fx {
        let theme = Theme::default();
        let styles = Styles::parse(Some(STYLES), &theme);
        let ctx = ColorCtx { theme: &theme, indexed: &styles.indexed };
        let sheet = sheet::parse(sheet_xml.as_bytes(), &ctx, &Rels::new(), false);
        let strings = Strings::parse(sst.as_bytes(), &ctx);
        Fx { sheet, styles, strings }
    }

    fn ws(body: &str) -> String {
        format!(r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{body}</worksheet>"#)
    }

    fn render(f: &Fx, part: usize) -> Value {
        let ctx = Ctx::new(&f.sheet, &f.styles, &f.strings, false);
        let plan = Plan::build(&ctx);
        let json = plan.render_part(&ctx, part, &[], "", f.sheet.no_grid);
        serde_json::from_str(&json).expect("valid json")
    }

    fn plan_of(f: &Fx) -> Plan {
        Plan::build(&Ctx::new(&f.sheet, &f.styles, &f.strings, false))
    }

    fn tag(v: &Value) -> &str {
        v["p"]["tag"].as_str().unwrap_or("")
    }

    fn class(v: &Value) -> &str {
        v["p"]["class"].as_str().unwrap_or("")
    }

    fn has(v: &Value, c: &str) -> bool {
        class(v).split(' ').any(|x| x == c)
    }

    fn text(v: &Value) -> &str {
        v["p"]["text"].as_str().unwrap_or("")
    }

    fn key(v: &Value) -> &str {
        v["p"]["key"].as_str().unwrap_or("")
    }

    /// The grid element of a part.
    fn grid(v: &Value) -> &Value {
        v["c"].as_array().unwrap().iter().find(|c| has(c, "xg")).unwrap()
    }

    /// The grid's children grouped per row: group 0 is the header (corner + letters), every other
    /// group starts with its row header, followed by that row's cells in document order.
    fn rows(v: &Value) -> Vec<Vec<&Value>> {
        let mut out: Vec<Vec<&Value>> = Vec::new();
        for (i, c) in grid(v)["c"].as_array().unwrap().iter().enumerate() {
            if i == 0 || has(c, "xr") {
                out.push(Vec::new());
            }
            out.last_mut().unwrap().push(c);
        }
        out
    }

    fn cells<'a>(row: &[&'a Value]) -> Vec<&'a Value> {
        row[1..].to_vec()
    }

    #[test]
    fn grid_shape_keys_and_actions() {
        let f = fx(
            &ws(r#"<sheetData><row r="1"><c r="A1" t="s"><v>0</v></c></row></sheetData>"#),
            r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>a"q"\b</t></si></sst>"#,
        );
        let v = render(&f, 0);
        assert_eq!(v["p"]["class"], "opv opx");
        let g = grid(&v);
        assert_eq!(tag(g), "div");
        assert_eq!(class(g), "xg gt2 ffh"); // row-number column + A, workbook default font
        let r = rows(&v);
        // corner: no actions; letter and row number: click actions and keys
        assert_eq!(*r[0][0], crate::node::Node::el("div").class("xc").prop("key", "x").into_value());
        let click = serde_json::json!({"click": "c"});
        assert_eq!((class(r[0][1]), key(r[0][1]), text(r[0][1])), ("xh w16", "h1", "A"));
        assert_eq!(r[0][1]["p"]["actions"], click);
        assert_eq!((class(r[1][0]), key(r[1][0]), text(r[1][0])), ("xr", "r1", "1"));
        assert_eq!(r[1][0]["p"]["actions"], click);
        // the cell, exactly
        let expect = crate::node::Node::el("div")
            .class("w16")
            .prop("key", "1:1")
            .prop("actions", click)
            .text("a\"q\"\\b")
            .into_value();
        assert_eq!(*r[1][1], expect);
        // no table tags or attrs anywhere
        let s = v.to_string();
        for bad in ["\"table\"", "\"td\"", "\"th\"", "\"tr\"", "colspan", "rowspan"] {
            assert!(!s.contains(bad), "{bad} in {s}");
        }
    }

    #[test]
    fn header_letters_widths_and_row_numbers() {
        let f = fx(
            &ws(
                r#"<sheetFormatPr defaultColWidth="9.140625"/><cols><col min="2" max="2" width="20" customWidth="1"/><col min="3" max="3" hidden="1" width="9"/></cols>
<sheetData><row r="2"><c r="A2"><v>1</v></c><c r="D2"><v>2</v></c></row><row r="4" hidden="1"><c r="A4"><v>9</v></c></row><row r="5" ht="30" customHeight="1"><c r="A5"><v>3</v></c></row></sheetData>"#,
            ),
            "<sst/>",
        );
        let v = render(&f, 0);
        assert_eq!(class(grid(&v)), "xg gt4 ffh");
        let r = rows(&v);
        let head: Vec<&str> = r[0].iter().map(|c| text(c)).collect();
        assert_eq!(head, vec!["", "A", "B", "D"]);
        let keys: Vec<&str> = r[0].iter().map(|c| key(c)).collect();
        assert_eq!(keys, vec!["x", "h1", "h2", "h4"]); // 1-based Excel columns; C is hidden
        assert_eq!(class(r[0][1]), "xh w16"); // 64 px
        assert_eq!(class(r[0][2]), "xh w35"); // 140 px → 35 × 4
        // rows 1, 2, 3 (blank), 5; row 4 hidden
        let nums: Vec<&str> = r[1..].iter().map(|row| text(row[0])).collect();
        assert_eq!(nums, vec!["1", "2", "3", "5"]);
        assert_eq!(cells(&r[1]).len(), 3);
        assert_eq!(key(cells(&r[2])[2]), "2:4");
        assert!(has(r[4][0], "h20"));
        assert!(has(cells(&r[4])[0], "h20"));
        // each cell carries its column's width
        assert!(has(cells(&r[1])[1], "w35") && has(cells(&r[1])[2], "w16"));
        assert_eq!(f.sheet.rows.len(), 3);
    }

    #[test]
    fn values_alignment_and_formats() {
        let f = fx(
            &ws(
                r#"<sheetData><row r="1"><c r="A1"><v>1234.5</v></c><c r="B1" t="b"><v>1</v></c><c r="C1" t="e"><v>#N/A</v></c><c r="D1" t="s"><v>0</v></c>
<c r="E1" s="4"><v>-2.25</v></c><c r="F1" s="5"><v>45000</v></c><c r="G1" s="1" t="s"><v>0</v></c></row></sheetData>"#,
            ),
            r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>文本</t></si></sst>"#,
        );
        let v = render(&f, 0);
        let c = cells(&rows(&v)[1]);
        assert_eq!((text(c[0]), has(c[0], "ar")), ("1234.5", true));
        assert_eq!((text(c[1]), has(c[1], "ac")), ("TRUE", true));
        assert_eq!((text(c[2]), has(c[2], "ac")), ("#N/A", true));
        assert_eq!(text(c[3]), "文本");
        assert!(!has(c[3], "ar") && !has(c[3], "ac"));
        // number format colour replaces the font colour
        assert_eq!(text(c[4]), "-2.3");
        assert!(has(c[4], "c700") && !has(c[4], "c007"));
        assert_eq!(text(c[5]), "2023/3/15");
        assert!(has(c[5], "ar"));
        assert!(has(c[6], "b") && has(c[6], "c007"));
    }

    #[test]
    fn fill_hides_gridlines_and_borders_are_normalised() {
        // A1 has a box border, B1 a red right border, C1 is filled, D1 plain.
        let f = fx(
            &ws(
                r#"<sheetData><row r="1"><c r="A1" s="3"><v>1</v></c><c r="B1" s="6"><v>2</v></c><c r="C1" s="2"><v>3</v></c><c r="D1"><v>4</v></c></row>
<row r="2"><c r="A2"><v>5</v></c><c r="B2" s="3"><v>6</v></c></row></sheetData>"#,
            ),
            "<sst/>",
        );
        let v = render(&f, 0);
        let r = rows(&v);
        let c1 = cells(&r[1]);
        // A1: own right/bottom + left/top (first column / first row)
        for k in ["br1", "bb1", "bl1", "bt1"] {
            assert!(has(c1[0], k), "A1 {k}: {}", class(c1[0]));
        }
        // B1: own medium red right, bottom comes from B2's top border
        assert!(has(c1[1], "br2") && has(c1[1], "bcr300"), "{}", class(c1[1]));
        assert!(has(c1[1], "bb1"), "{}", class(c1[1]));
        assert!(!has(c1[1], "bl1") && !has(c1[1], "bt1"));
        // C1 is filled: no gridline right (D1 plain, own none) or below
        assert!(has(c1[2], "brn") && has(c1[2], "bbn"), "{}", class(c1[2]));
        // D1: next to a filled cell on its left, but its own edges are plain
        assert!(!has(c1[3], "brn"));
        let c2 = cells(&r[2]);
        // A2: right edge = B2's left border; no top (A1's bottom is drawn by A1)
        assert!(has(c2[0], "br1") && !has(c2[0], "bt1") && !has(c2[0], "bl1"), "{}", class(c2[0]));
        // B2: own box on right/bottom; its top was handed to B1
        assert!(has(c2[1], "br1") && has(c2[1], "bb1"));
    }

    #[test]
    fn no_gridlines_class_goes_on_the_grid() {
        let f = fx(
            &ws(
                r#"<sheetViews><sheetView showGridLines="0"/></sheetViews><sheetData><row r="1"><c r="A1" s="2"><v>1</v></c></row></sheetData>"#,
            ),
            "<sst/>",
        );
        let v = render(&f, 0);
        assert_eq!(v["p"]["class"], "opv opx");
        assert_eq!(class(grid(&v)), "xg gt2 ffh opxng"); // default font class, then opxng
        let c = cells(&rows(&v)[1]);
        assert!(!has(c[0], "brn") && !has(c[0], "bbn"));
    }

    #[test]
    fn merges_become_span_classes_and_omit_covered_cells() {
        // A1:B2 merged; C1:C3 merged; E2:F2 merged (E column hidden → one visible column).
        let f = fx(
            &ws(r#"<cols><col min="5" max="5" hidden="1"/></cols><sheetData>
<row r="1" ht="30" customHeight="1"><c r="A1" s="3" t="s"><v>0</v></c><c r="B1" s="3"/><c r="C1" t="s"><v>0</v></c></row>
<row r="2"><c r="A2" s="3"/><c r="B2" s="3"/><c r="E2"><v>5</v></c><c r="F2"><v>6</v></c></row>
<row r="3"><c r="A3"><v>7</v></c><c r="B3"><v>8</v></c></row></sheetData>
<mergeCells count="3"><mergeCell ref="A1:B2"/><mergeCell ref="C1:C3"/><mergeCell ref="E2:F2"/></mergeCells>"#),
            r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>m</t></si></sst>"#,
        );
        let v = render(&f, 0);
        let r = rows(&v);
        // header: corner + A B C D F (E hidden)
        assert_eq!(r[0].len(), 6);
        assert_eq!(class(grid(&v)), "xg gt6 ffh");
        let c1 = cells(&r[1]);
        assert_eq!(c1.len(), 4, "row 1: merged A1:B2, merged C1:C3, D1, F1");
        // A1:B2: two columns (w32 = 2 × 64 px) and two rows; stretches, so no fixed height
        for k in ["gs2", "gr2", "w32"] {
            assert!(has(c1[0], k), "{k}: {}", class(c1[0]));
        }
        assert!(!class(c1[0]).split(' ').any(|c| c.starts_with('h')), "{}", class(c1[0]));
        assert_eq!((text(c1[0]), key(c1[0])), ("m", "1:1"));
        // C1:C3: three rows, one column
        assert!(has(c1[1], "gr3") && has(c1[1], "w16") && !class(c1[1]).contains("gs"), "{}", class(c1[1]));
        assert_eq!(key(c1[1]), "1:3");
        // an unmerged cell of the 30pt row keeps the fixed height
        assert!(has(c1[2], "h20"), "{}", class(c1[2]));
        // row 2: A..C are covered; D2 and the merged E2:F2 (one visible column, no span) remain
        let c2 = cells(&r[2]);
        assert_eq!(c2.len(), 2, "D2 and the merged E2:F2");
        assert_eq!(text(c2[1]), "5"); // the hidden E2 still supplies the merged text
        assert!(!class(c2[1]).contains("gs") && !class(c2[1]).contains("gr"));
        assert_eq!(key(c2[1]), "2:6");
        // row 3: A3, B3 (C covered), D3, F3
        let c3 = cells(&r[3]);
        assert_eq!(c3.len(), 4);
        assert_eq!(text(c3[0]), "7");
        // The merged A1:B2 uses its right column / bottom row edges: box border all around.
        assert!(has(c1[0], "br1") && has(c1[0], "bb1") && has(c1[0], "bl1") && has(c1[0], "bt1"), "{}", class(c1[0]));
    }

    #[test]
    fn text_overflow_spills_into_empty_cells() {
        let long = "x".repeat(40);
        let sst = format!(
            r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>{long}</t></si><si><t>短</t></si></sst>"#
        );
        let f = fx(
            &ws(r#"<sheetData>
<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1"/><c r="C1"/><c r="D1" t="s"><v>1</v></c><c r="E1" t="s"><v>1</v></c></row>
<row r="2"><c r="A2" t="s"><v>0</v></c><c r="B2" s="2"/><c r="C2" t="s"><v>1</v></c></row>
<row r="3"><c r="A3" s="7" t="s"><v>0</v></c><c r="B3"/><c r="C3"><v>1</v></c></row>
<row r="4"><c r="A4" t="s"><v>1</v></c><c r="B4"><v>1</v></c><c r="E4"><v>1</v></c></row></sheetData>"#),
            &sst,
        );
        let v = render(&f, 0);
        let r = rows(&v);
        // 40 chars × 14.67 × 0.55 ≈ 322 px: A+B+C (64 each → 192, +D) — spills until D (non-empty)
        let c1 = cells(&r[1]);
        assert!(has(c1[0], "gs3"), "{}", class(c1[0]));
        assert_eq!(c1.len(), 3);
        // still wider than A:C, so it is clipped there: pinned to their width, full text as tooltip
        assert!(has(c1[0], "w48"), "{}", class(c1[0]));
        assert_eq!(c1[0]["p"]["title"], long.as_str());
        assert_eq!(key(c1[1]), "1:4");
        // a filled neighbour blocks the spill; the text is clipped to A
        let c2 = cells(&r[2]);
        assert!(!class(c2[0]).contains("gs"));
        assert_eq!(c2.len(), 5);
        assert!(has(c2[0], "w16"), "{}", class(c2[0]));
        // wrapped / right aligned text never spills
        let c3 = cells(&r[3]);
        assert!(!class(c3[0]).contains("gs"));
        // short text stays in its cell: one column wide, no tooltip
        let c4 = cells(&r[4]);
        assert!(has(c4[0], "w16") && !class(c4[0]).contains("gs"), "{}", class(c4[0]));
        assert!(c4[0]["p"].get("title").is_none());
    }

    #[test]
    fn rich_text_runs_share_one_span() {
        let f = fx(
            &ws(r#"<sheetData><row r="1"><c r="A1" t="s"><v>0</v></c></row></sheetData>"#),
            r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><r><rPr><b/><color rgb="FFFF0000"/></rPr><t>Hot</t></r><r><t> day</t></r></si></sst>"#,
        );
        let v = render(&f, 0);
        let cell = cells(&rows(&v)[1])[0];
        assert!(cell["p"].get("text").is_none());
        let wrap = cell["c"].as_array().unwrap();
        assert_eq!(wrap.len(), 1);
        assert_eq!(tag(&wrap[0]), "span");
        let spans = wrap[0]["c"].as_array().unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!((text(&spans[0]), class(&spans[0])), ("Hot", "b c700"));
        assert_eq!((text(&spans[1]), class(&spans[1])), (" day", ""));
    }

    #[test]
    fn newlines_follow_wrap() {
        let f = fx(
            &ws(
                r#"<sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" s="7" t="s"><v>0</v></c></row></sheetData>"#,
            ),
            r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t xml:space="preserve">a
b</t></si></sst>"#,
        );
        let v = render(&f, 0);
        let c = cells(&rows(&v)[1]);
        assert_eq!(text(c[0]), "ab");
        assert_eq!(text(c[1]), "a\nb");
        assert!(has(c[1], "ww") && has(c[1], "ar"));
    }

    fn big_sheet(nrows: u32, ncols: u32, frozen: u32) -> Fx {
        let mut body = String::new();
        if frozen > 0 {
            body.push_str(&format!(
                r#"<sheetViews><sheetView><pane ySplit="{frozen}" state="frozen"/></sheetView></sheetViews>"#
            ));
        }
        body.push_str("<sheetData>");
        for r in 1..=nrows {
            body.push_str(&format!(r#"<row r="{r}">"#));
            for c in 0..ncols {
                body.push_str(&format!(r#"<c r="{}{r}"><v>{}</v></c>"#, col_letters(c), r * 1000 + c));
            }
            body.push_str("</row>");
        }
        body.push_str("</sheetData>");
        fx(&ws(&body), "<sst/>")
    }

    #[test]
    fn parts_split_by_rows_and_repeat_frozen_rows() {
        let f = big_sheet(1000, 10, 2);
        let plan = plan_of(&f);
        // 5000 / 10 columns = 500 rows per part
        assert_eq!(plan.parts.len(), 2);
        assert_eq!(plan.label(0), "1–500");
        assert_eq!(plan.label(1), "501–1000");
        assert_eq!(plan.frozen, 2);
        let v0 = render(&f, 0);
        let v1 = render(&f, 1);
        let r0 = rows(&v0);
        let r1 = rows(&v1);
        assert_eq!(r0.len(), 1 + 500);
        // part 1: header + 2 frozen rows + 500 rows
        assert_eq!(r1.len(), 1 + 2 + 500);
        let nums: Vec<&str> = r1[1..5].iter().map(|row| text(row[0])).collect();
        assert_eq!(nums, vec!["1", "2", "501", "502"]);
        // keys keep the real Excel row numbers, also for the repeated frozen rows
        assert_eq!(key(cells(&r1[3])[0]), "501:1");
        assert_eq!(key(cells(&r1[1])[0]), "1:1");
        // last frozen row separates; also in part 0
        assert!(has(cells(&r1[2])[0], "fz"));
        assert!(!has(cells(&r1[1])[0], "fz"));
        assert!(has(cells(&r0[2])[0], "fz"));
        // node budget per part
        let count = |v: &Value| -> usize { v.to_string().matches(r#""k":"el""#).count() };
        assert!(count(&v0) <= PART_NODES, "{}", count(&v0));
    }

    #[test]
    fn wide_sheets_get_fewer_rows_per_part() {
        let f = big_sheet(300, 100, 0);
        let plan = plan_of(&f);
        assert_eq!(plan.vcols.len(), 100);
        assert_eq!(plan.parts[0].len(), 50);
        assert_eq!(plan.parts.len(), 6);
        let f = big_sheet(100, 300, 0);
        let plan = plan_of(&f);
        assert_eq!(plan.vcols.len(), MAX_COLS as usize);
        // 256 columns: the node budget wins over the 40-row minimum.
        assert_eq!(plan.parts[0].len(), PART_NODES / (MAX_COLS as usize + 1));
        assert!(plan.notes.iter().any(|n| n.contains("256 列")), "{:?}", plan.notes);
    }

    #[test]
    fn merge_continues_across_part_boundaries() {
        // 100 columns → 50 rows per part; A49:A52 merged spans parts 0 and 1.
        let mut body = String::from("<sheetData>");
        for r in 1..=60 {
            body.push_str(&format!(r#"<row r="{r}">"#));
            for c in 0..100 {
                body.push_str(&format!(r#"<c r="{}{r}"><v>1</v></c>"#, col_letters(c)));
            }
            body.push_str("</row>");
        }
        body.push_str(r#"</sheetData><mergeCells count="1"><mergeCell ref="A49:A52"/></mergeCells>"#);
        let f = fx(&ws(&body), "<sst/>");
        let v0 = render(&f, 0);
        let v1 = render(&f, 1);
        let r0 = rows(&v0);
        let r1 = rows(&v1);
        // part 0: rows 49, 50 → two rows, with the real content
        let c = cells(&r0[49]);
        assert!(has(c[0], "gr2"), "{}", class(c[0]));
        assert_eq!((text(c[0]), key(c[0])), ("1", "49:1"));
        assert_eq!(cells(&r0[50]).len(), 99);
        // part 1: rows 51, 52 → continuation: empty cell spanning 2 rows, row 52 has 99 cells
        let c = cells(&r1[1]);
        assert!(has(c[0], "gr2"), "{}", class(c[0]));
        assert_eq!((text(c[0]), key(c[0])), ("", "51:1"));
        assert_eq!(cells(&r1[1]).len(), 100);
        assert_eq!(cells(&r1[2]).len(), 99);
    }

    #[test]
    fn row_and_column_styles_fill_absent_cells() {
        let f = fx(
            &ws(
                r#"<cols><col min="2" max="2" width="9" style="2"/></cols><sheetData><row r="1"><c r="A1"><v>1</v></c><c r="C1"><v>1</v></c></row>
<row r="2" s="3" customFormat="1"><c r="A2"><v>1</v></c><c r="C2"><v>1</v></c></row></sheetData>"#,
            ),
            "<sst/>",
        );
        let v = render(&f, 0);
        let r = rows(&v);
        let c1 = cells(&r[1]);
        assert!(has(c1[1], "k770"), "column style fill: {}", class(c1[1]));
        let c2 = cells(&r[2]);
        assert!(has(c2[1], "bb1") && has(c2[1], "br1"), "row style border: {}", class(c2[1]));
    }

    #[test]
    fn hyperlinks_carry_href() {
        let theme = Theme::default();
        let styles = Styles::parse(Some(STYLES), &theme);
        let ctx = ColorCtx { theme: &theme, indexed: &styles.indexed };
        let mut rels = Rels::new();
        rels.insert(
            "rId1".into(),
            crate::pkg::Rel { kind: "hyperlink".into(), target: "https://example.com/x".into(), external: true },
        );
        let sheet = sheet::parse(
            ws(r#"<sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>2</v></c></row></sheetData><hyperlinks><hyperlink ref="A1" r:id="rId1" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/></hyperlinks>"#).as_bytes(),
            &ctx,
            &rels,
            false,
        );
        let strings = Strings::default();
        let c = Ctx::new(&sheet, &styles, &strings, false);
        let plan = Plan::build(&c);
        let v: Value = serde_json::from_str(&plan.render_part(&c, 0, &[], "", false)).unwrap();
        let td = cells(&rows(&v)[1]);
        assert_eq!(td[0]["p"]["href"], "https://example.com/x");
        assert!(has(td[0], "lnk"));
        assert!(td[1]["p"].get("href").is_none());
    }

    #[test]
    fn notes_and_extra_nodes() {
        let f = fx(&ws(r#"<sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData>"#), "<sst/>");
        let ctx = Ctx::new(&f.sheet, &f.styles, &f.strings, false);
        let plan = Plan::build(&ctx);
        let json = plan.render_part(
            &ctx,
            0,
            &["含条件格式（未显示）".to_string()],
            r#"{"k":"el","p":{"tag":"div","class":"xs"}}"#,
            false,
        );
        let v: Value = serde_json::from_str(&json).unwrap();
        let kids = v["c"].as_array().unwrap();
        assert_eq!(class(&kids[0]), "xmsg");
        assert!(has(&kids[1], "xg"));
        assert_eq!(class(&kids[2]), "xs");
    }

    #[test]
    fn sheet_without_cells_has_no_parts() {
        let f = fx(&ws("<sheetData/>"), "<sst/>");
        let plan = plan_of(&f);
        assert!(plan.parts.is_empty());
    }

    #[test]
    fn push_text_escapes() {
        let mut s = String::new();
        push_text(&mut s, "a\"b\\c\nd\te\r\u{1}f\u{2028}", false);
        assert_eq!(s, "a\\\"b\\\\cd\\tef\\u2028");
        let mut s = String::new();
        push_text(&mut s, "x\ny", true);
        assert_eq!(s, "x\\ny");
    }

    #[test]
    fn widths_and_overflow_estimate() {
        assert_eq!(width_px(9.140625), 64.0);
        assert_eq!(width_px(20.0), 140.0);
        assert_eq!(quant(63.0), 64.0);
        assert!((text_width("abcd", 10.0) - 22.0).abs() < 1e-9);
        assert!((text_width("中文", 10.0) - 22.0).abs() < 1e-9);
    }
}
