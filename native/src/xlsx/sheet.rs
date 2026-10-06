//! Worksheet XML, streamed with quick-xml into a compact row-major model (sheets can be 100+ MB).

use super::numfmt;
use super::strings::{self, Run};
use super::styles::ColorCtx;
use crate::pkg::Rels;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

/// Rows beyond this number are not read into the model.
pub const MAX_ROWS: u32 = 100_000;
/// Columns beyond this (0-based index) are not read into the model.
pub const MAX_COLS: u32 = 256;
/// Marks "no style" in `Row::xf`.
pub const NO_XF: u16 = u16::MAX;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Val {
    Empty,
    Num(f64),
    /// An ISO date (`t="d"`) as a serial number.
    Date(f64),
    Shared(u32),
    /// Offset and length in `Sheet::arena`.
    Text(u32, u32),
    /// Index in `Sheet::rich`.
    Rich(u32),
    Bool(bool),
    Err(u32, u32),
}

#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub col: u16,
    pub xf: u16,
    pub val: Val,
}

#[derive(Clone, Copy, Debug)]
pub struct Row {
    /// 1-based row number.
    pub r: u32,
    pub start: u32,
    pub end: u32,
    /// Height in points; 0 when unset.
    pub ht: f32,
    pub hidden: bool,
    /// Row style when `customFormat` is set.
    pub xf: u16,
}

#[derive(Clone, Debug)]
pub struct ColDef {
    /// 0-based, inclusive.
    pub min: u32,
    pub max: u32,
    /// Excel width units.
    pub width: Option<f64>,
    pub hidden: bool,
    pub xf: u16,
}

/// 1-based rows, 0-based columns, inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Merge {
    pub r0: u32,
    pub c0: u32,
    pub r1: u32,
    pub c1: u32,
}

#[derive(Clone, Debug)]
pub struct Link {
    pub range: Merge,
    pub url: String,
}

#[derive(Default)]
pub struct Sheet {
    pub rows: Vec<Row>,
    pub cells: Vec<Cell>,
    pub arena: String,
    pub rich: Vec<Vec<Run>>,
    pub cols: Vec<ColDef>,
    pub merges: Vec<Merge>,
    pub links: Vec<Link>,
    pub default_col_w: Option<f64>,
    pub base_col_w: Option<f64>,
    /// Default row height in points, when the sheet sets one explicitly.
    pub default_row_ht: Option<f64>,
    pub frozen_rows: u32,
    pub no_grid: bool,
    /// Highest row / column (count) holding a value, beyond what was read into the model.
    pub over_rows: u32,
    pub over_cols: u32,
    pub has_cf: bool,
    pub drawing_rid: Option<String>,
}

impl Sheet {
    pub fn cells_of(&self, row: &Row) -> &[Cell] {
        &self.cells[row.start as usize..row.end as usize]
    }

    pub fn str_at(&self, off: u32, len: u32) -> &str {
        self.arena.get(off as usize..(off + len) as usize).unwrap_or("")
    }

    fn put(&mut self, s: &str) -> (u32, u32) {
        let off = self.arena.len() as u32;
        self.arena.push_str(s);
        (off, s.len() as u32)
    }
}

/// `A` → 0, `AA` → 26.
pub fn col_index(letters: &[u8]) -> Option<u32> {
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let mut n = 0u32;
    for b in letters {
        if !b.is_ascii_alphabetic() {
            return None;
        }
        n = n * 26 + (b.to_ascii_uppercase() - b'A') as u32 + 1;
    }
    Some(n - 1)
}

/// 0 → `A`, 26 → `AA`.
pub fn col_letters(mut i: u32) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// `B4` → (row 4, col 1).
pub fn parse_ref(b: &[u8]) -> Option<(u32, u32)> {
    let split = b.iter().position(|c| c.is_ascii_digit())?;
    let col = col_index(&b[..split])?;
    let row: u32 = std::str::from_utf8(&b[split..]).ok()?.parse().ok()?;
    if row == 0 {
        return None;
    }
    Some((row, col))
}

fn parse_range(s: &str) -> Option<Merge> {
    let (a, b) = s.split_once(':').unwrap_or((s, s));
    let (r0, c0) = parse_ref(a.trim().as_bytes())?;
    let (r1, c1) = parse_ref(b.trim().as_bytes())?;
    Some(Merge { r0: r0.min(r1), c0: c0.min(c1), r1: r0.max(r1), c1: c0.max(c1) })
}

fn pnum<T: std::str::FromStr>(b: &[u8]) -> Option<T> {
    std::str::from_utf8(b).ok()?.trim().parse().ok()
}

fn truthy(b: &[u8]) -> bool {
    matches!(b, b"1" | b"true")
}

/// `2023-03-15T10:20:30Z`, `2023-03-15`, `10:20:30` → serial number.
pub fn parse_iso(s: &str, date1904: bool) -> Option<f64> {
    let s = s.trim();
    let (date, time) = match s.split_once(['T', ' ']) {
        Some((d, t)) => (d, Some(t)),
        None if s.contains(':') => ("", Some(s)),
        None => (s, None),
    };
    let mut serial = 0.0;
    if !date.is_empty() {
        let mut it = date.splitn(3, '-');
        let y: i64 = it.next()?.parse().ok()?;
        let m: i64 = it.next()?.parse().ok()?;
        let d: i64 = it.next()?.parse().ok()?;
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        serial = numfmt::serial_from_ymd(y, m, d, date1904);
    }
    if let Some(t) = time {
        let t = t.trim_end_matches('Z');
        let t = t.split(['+', '-']).next().unwrap_or(t);
        let mut it = t.split(':');
        let h: f64 = it.next()?.parse().ok()?;
        let m: f64 = it.next().map(|x| x.parse().ok()).unwrap_or(Some(0.0))?;
        let sec: f64 = it.next().map(|x| x.parse().ok()).unwrap_or(Some(0.0))?;
        serial += (h * 3600.0 + m * 60.0 + sec) / 86400.0;
    }
    Some(serial)
}

#[derive(Clone, Copy, PartialEq)]
enum T {
    N,
    S,
    Str,
    B,
    E,
    D,
    Inline,
}

struct CellSt {
    col: u32,
    xf: u16,
    t: T,
    inline: Option<strings::Si>,
}

struct RowSt {
    row: Row,
    keep: bool,
    unsorted: bool,
}

struct Parser<'c, 'r> {
    sh: Sheet,
    ctx: &'c ColorCtx<'c>,
    rels: &'r Rels,
    date1904: bool,
    row: Option<RowSt>,
    cell: Option<CellSt>,
    last_row: u32,
    next_col: u32,
    in_v: bool,
    in_f: bool,
    v: String,
    f: String,
    seen_view: bool,
}

/// Parses worksheet XML.
pub fn parse(bytes: &[u8], ctx: &ColorCtx, rels: &Rels, date1904: bool) -> Sheet {
    let mut p = Parser {
        sh: Sheet::default(),
        ctx,
        rels,
        date1904,
        row: None,
        cell: None,
        last_row: 0,
        next_col: 0,
        in_v: false,
        in_f: false,
        v: String::new(),
        f: String::new(),
        seen_view: false,
    };
    let mut r = Reader::from_reader(bytes);
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => p.start(&mut r, &e, false),
            Ok(Event::Empty(e)) => p.start(&mut r, &e, true),
            Ok(Event::End(e)) => p.end(e.local_name().as_ref()),
            Ok(Event::Text(t)) => {
                if p.in_v {
                    match t.unescape() {
                        Ok(s) => p.v.push_str(&s),
                        Err(_) => p.v.push_str(&String::from_utf8_lossy(&t)),
                    }
                } else if p.in_f && p.f.len() < 4096 {
                    match t.unescape() {
                        Ok(s) => p.f.push_str(&s),
                        Err(_) => p.f.push_str(&String::from_utf8_lossy(&t)),
                    }
                }
            }
            Ok(Event::CData(t)) => {
                if p.in_v {
                    p.v.push_str(&String::from_utf8_lossy(&t));
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    p.finish_row();
    p.sh
}

impl Parser<'_, '_> {
    fn start(&mut self, r: &mut Reader<&[u8]>, e: &BytesStart, empty: bool) {
        match e.local_name().as_ref() {
            b"row" => {
                self.finish_row();
                let mut row = Row {
                    r: self.last_row + 1,
                    start: self.sh.cells.len() as u32,
                    end: 0,
                    ht: 0.0,
                    hidden: false,
                    xf: NO_XF,
                };
                let mut custom_fmt = false;
                let mut s = NO_XF;
                for a in e.attributes().with_checks(false).flatten() {
                    let v = a.value.as_ref();
                    match a.key.local_name().as_ref() {
                        b"r" => {
                            if let Some(n) = pnum(v) {
                                row.r = n;
                            }
                        }
                        b"ht" => row.ht = pnum(v).unwrap_or(0.0),
                        b"hidden" => row.hidden = truthy(v),
                        b"s" => s = pnum::<u32>(v).map(|n| n.min(NO_XF as u32 - 1) as u16).unwrap_or(NO_XF),
                        b"customFormat" => custom_fmt = truthy(v),
                        _ => {}
                    }
                }
                if custom_fmt {
                    row.xf = s;
                }
                self.last_row = row.r;
                self.next_col = 0;
                self.row = Some(RowSt { keep: row.r <= MAX_ROWS, row, unsorted: false });
                if empty {
                    self.finish_row();
                }
            }
            b"c" => {
                let mut c = CellSt { col: self.next_col, xf: 0, t: T::N, inline: None };
                for a in e.attributes().with_checks(false).flatten() {
                    let v = a.value.as_ref();
                    match a.key.local_name().as_ref() {
                        b"r" => {
                            if let Some((_, col)) = parse_ref(v) {
                                c.col = col;
                            }
                        }
                        b"s" => c.xf = pnum::<u32>(v).map(|n| n.min(NO_XF as u32 - 1) as u16).unwrap_or(0),
                        b"t" => {
                            c.t = match v {
                                b"s" => T::S,
                                b"str" => T::Str,
                                b"b" => T::B,
                                b"e" => T::E,
                                b"d" => T::D,
                                b"inlineStr" => T::Inline,
                                _ => T::N,
                            }
                        }
                        _ => {}
                    }
                }
                self.next_col = c.col + 1;
                self.v.clear();
                self.f.clear();
                self.cell = Some(c);
                if empty {
                    self.finish_cell();
                }
            }
            b"v" if self.cell.is_some() => self.in_v = !empty,
            b"f" if self.cell.is_some() => self.in_f = !empty,
            b"is" if self.cell.is_some() && !empty => {
                let si = strings::read_si(r, self.ctx, b"is");
                if let Some(c) = &mut self.cell {
                    c.inline = Some(si);
                }
            }
            b"sheetFormatPr" => {
                let mut custom = false;
                let mut ht = None;
                for a in e.attributes().with_checks(false).flatten() {
                    let v = a.value.as_ref();
                    match a.key.local_name().as_ref() {
                        b"defaultColWidth" => self.sh.default_col_w = pnum(v),
                        b"baseColWidth" => self.sh.base_col_w = pnum(v),
                        b"defaultRowHeight" => ht = pnum(v),
                        b"customHeight" => custom = truthy(v),
                        _ => {}
                    }
                }
                if custom {
                    self.sh.default_row_ht = ht;
                }
            }
            b"sheetView" if !self.seen_view => {
                self.seen_view = true;
                for a in e.attributes().with_checks(false).flatten() {
                    if a.key.local_name().as_ref() == b"showGridLines" && !truthy(a.value.as_ref()) {
                        self.sh.no_grid = true;
                    }
                }
            }
            b"pane" if self.sh.frozen_rows == 0 => {
                let mut y = 0.0f64;
                let mut frozen = false;
                for a in e.attributes().with_checks(false).flatten() {
                    let v = a.value.as_ref();
                    match a.key.local_name().as_ref() {
                        b"ySplit" => y = pnum(v).unwrap_or(0.0),
                        b"state" => frozen = v.starts_with(b"frozen"),
                        _ => {}
                    }
                }
                if frozen && y >= 1.0 {
                    self.sh.frozen_rows = y as u32;
                }
            }
            b"col" => {
                let mut d = ColDef { min: 0, max: 0, width: None, hidden: false, xf: NO_XF };
                for a in e.attributes().with_checks(false).flatten() {
                    let v = a.value.as_ref();
                    match a.key.local_name().as_ref() {
                        b"min" => d.min = pnum::<u32>(v).unwrap_or(1).max(1) - 1,
                        b"max" => d.max = pnum::<u32>(v).unwrap_or(1).max(1) - 1,
                        b"width" => d.width = pnum(v),
                        b"hidden" => d.hidden = truthy(v),
                        b"style" => d.xf = pnum::<u32>(v).map(|n| n.min(NO_XF as u32 - 1) as u16).unwrap_or(NO_XF),
                        _ => {}
                    }
                }
                if d.max >= d.min && self.sh.cols.len() < 20_000 {
                    self.sh.cols.push(d);
                }
            }
            b"mergeCell" if self.sh.merges.len() < 200_000 => {
                for a in e.attributes().with_checks(false).flatten() {
                    if a.key.local_name().as_ref() == b"ref" {
                        if let Some(m) = std::str::from_utf8(a.value.as_ref()).ok().and_then(parse_range) {
                            if m.r0 != m.r1 || m.c0 != m.c1 {
                                self.sh.merges.push(m);
                            }
                        }
                    }
                }
            }
            b"conditionalFormatting" => self.sh.has_cf = true,
            b"drawing" => {
                for a in e.attributes().with_checks(false).flatten() {
                    if a.key.local_name().as_ref() == b"id" {
                        self.sh.drawing_rid = Some(String::from_utf8_lossy(a.value.as_ref()).into_owned());
                    }
                }
            }
            b"hyperlink" => {
                let mut range = None;
                let mut rid = None;
                for a in e.attributes().with_checks(false).flatten() {
                    match a.key.local_name().as_ref() {
                        b"ref" => range = std::str::from_utf8(a.value.as_ref()).ok().and_then(parse_range),
                        b"id" => rid = Some(String::from_utf8_lossy(a.value.as_ref()).into_owned()),
                        _ => {}
                    }
                }
                if let (Some(range), Some(rid)) = (range, rid) {
                    if let Some(rel) = self.rels.get(&rid) {
                        if rel.external && self.sh.links.len() < 100_000 {
                            self.sh.links.push(Link { range, url: rel.target.clone() });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn end(&mut self, name: &[u8]) {
        match name {
            b"v" => self.in_v = false,
            b"f" => self.in_f = false,
            b"c" => self.finish_cell(),
            b"row" => self.finish_row(),
            _ => {}
        }
    }

    fn finish_cell(&mut self) {
        self.in_v = false;
        self.in_f = false;
        let Some(c) = self.cell.take() else { return };
        let Some(row) = &self.row else { return };
        let r = row.row.r;
        if r > MAX_ROWS || c.col >= MAX_COLS {
            if c.inline.is_some() || !self.v.trim().is_empty() || !self.f.trim().is_empty() {
                if r > MAX_ROWS {
                    self.sh.over_rows = self.sh.over_rows.max(r);
                }
                if c.col >= MAX_COLS {
                    self.sh.over_cols = self.sh.over_cols.max(c.col + 1);
                }
            }
            return;
        }
        let val = self.make_val(&c);
        if c.xf == 0 && val == Val::Empty {
            return;
        }
        let col = c.col as u16;
        if let Some(row) = &mut self.row {
            if self.sh.cells.len() as u32 > row.row.start && self.sh.cells.last().is_some_and(|l| l.col >= col) {
                row.unsorted = true;
            }
        }
        self.sh.cells.push(Cell { col, xf: c.xf, val });
    }

    fn make_val(&mut self, c: &CellSt) -> Val {
        if c.t == T::Inline {
            if let Some(si) = &c.inline {
                if !si.runs.is_empty() {
                    self.sh.rich.push(si.runs.clone());
                    return Val::Rich(self.sh.rich.len() as u32 - 1);
                }
                if si.text.is_empty() {
                    return Val::Empty;
                }
                let (o, l) = self.sh.put(&si.text);
                return Val::Text(o, l);
            }
        }
        let v = std::mem::take(&mut self.v);
        let t = v.trim();
        if t.is_empty() {
            // A formula that was never calculated: show it.
            let f = self.f.trim();
            if !f.is_empty() {
                let s = format!("={f}");
                let (o, l) = self.sh.put(&s);
                return Val::Text(o, l);
            }
            return Val::Empty;
        }
        match c.t {
            T::S => t.parse::<u32>().map(Val::Shared).unwrap_or(Val::Empty),
            T::B => Val::Bool(t == "1" || t.eq_ignore_ascii_case("true")),
            T::E => {
                let (o, l) = self.sh.put(t);
                Val::Err(o, l)
            }
            T::Str | T::Inline => {
                let s = strings::unescape_x(v.clone());
                let (o, l) = self.sh.put(&s);
                Val::Text(o, l)
            }
            T::D => match parse_iso(t, self.date1904) {
                Some(s) => Val::Date(s),
                None => {
                    let (o, l) = self.sh.put(t);
                    Val::Text(o, l)
                }
            },
            T::N => match t.parse::<f64>() {
                Ok(n) if n.is_finite() => Val::Num(n),
                _ => {
                    let (o, l) = self.sh.put(t);
                    Val::Text(o, l)
                }
            },
        }
    }

    fn finish_row(&mut self) {
        let Some(mut st) = self.row.take() else { return };
        if !st.keep {
            return;
        }
        let start = st.row.start as usize;
        if st.unsorted {
            let tail = &mut self.sh.cells[start..];
            tail.sort_by_key(|c| c.col);
            // Keep the last of duplicate columns.
            let mut w = 0;
            for i in 0..tail.len() {
                if i + 1 < tail.len() && tail[i].col == tail[i + 1].col {
                    continue;
                }
                tail[w] = tail[i];
                w += 1;
            }
            self.sh.cells.truncate(start + w);
        }
        st.row.end = self.sh.cells.len() as u32;
        // Rows must ascend; a stray out-of-order row is dropped rather than misplaced.
        if self.sh.rows.last().is_some_and(|l| l.r >= st.row.r) {
            self.sh.cells.truncate(start);
            return;
        }
        let has_attrs = st.row.ht > 0.0 || st.row.hidden || st.row.xf != NO_XF;
        if st.row.end > st.row.start || has_attrs {
            self.sh.rows.push(st.row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Theme;
    use crate::pkg::Rel;

    fn parse_xml(xml: &str, rels: &Rels) -> Sheet {
        let theme = Theme::default();
        let ctx = ColorCtx { theme: &theme, indexed: &[] };
        parse(xml.as_bytes(), &ctx, rels, false)
    }

    #[test]
    fn refs_and_letters() {
        assert_eq!(parse_ref(b"A1"), Some((1, 0)));
        assert_eq!(parse_ref(b"AA10"), Some((10, 26)));
        assert_eq!(parse_ref(b"XFD1048576"), Some((1048576, 16383)));
        assert_eq!(parse_ref(b"A0"), None);
        assert_eq!(parse_ref(b"11"), None);
        assert_eq!(col_letters(0), "A");
        assert_eq!(col_letters(25), "Z");
        assert_eq!(col_letters(26), "AA");
        assert_eq!(col_letters(27), "AB");
        assert_eq!(col_letters(701), "ZZ");
        assert_eq!(col_letters(702), "AAA");
        for i in [0, 1, 25, 26, 51, 52, 255, 701, 702, 16383] {
            assert_eq!(col_index(col_letters(i).as_bytes()), Some(i));
        }
        assert_eq!(parse_range("B2:C4"), Some(Merge { r0: 2, c0: 1, r1: 4, c1: 2 }));
        assert_eq!(parse_range("C4:B2"), Some(Merge { r0: 2, c0: 1, r1: 4, c1: 2 }));
    }

    #[test]
    fn iso_dates() {
        assert_eq!(parse_iso("2023-03-15", false), Some(45000.0));
        assert_eq!(parse_iso("2023-03-15T12:00:00Z", false), Some(45000.5));
        assert_eq!(parse_iso("06:00:00", false), Some(0.25));
        assert_eq!(parse_iso("1904-01-01", true), Some(0.0));
        assert_eq!(parse_iso("garbage", false), None);
    }

    #[test]
    fn rows_cells_and_metadata() {
        let mut rels = Rels::new();
        rels.insert(
            "rId1".into(),
            Rel { kind: "hyperlink".into(), target: "https://example.com/a?b=1&c=2".into(), external: true },
        );
        let sh = parse_xml(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheetViews><sheetView showGridLines="0"><pane ySplit="2" state="frozen" topLeftCell="A3"/></sheetView></sheetViews>
<sheetFormatPr defaultRowHeight="18" customHeight="1" defaultColWidth="10.5"/>
<cols><col min="1" max="2" width="12.7" customWidth="1"/><col min="4" max="4" hidden="1" width="9" style="3"/></cols>
<sheetData>
<row r="1" ht="30" customHeight="1"><c r="A1" s="2" t="s"><v>5</v></c><c r="B1"><v>3.5</v></c><c r="C1" t="b"><v>1</v></c></row>
<row r="3" hidden="1" s="4" customFormat="1"><c r="A3" t="str"><f>A1&amp;B1</f><v>x &amp; y</v></c><c r="B3" t="e"><v>#DIV/0!</v></c></row>
<row r="4"><c r="A4"><f>SUM(A1:A3)</f><v></v></c><c r="B4" t="inlineStr"><is><t>inline</t></is></c><c r="C4" s="1"/><c r="D4" t="d"><v>2023-03-15</v></c></row>
<row r="5"><c r="E5" t="inlineStr"><is><r><rPr><b/></rPr><t>B</t></r><r><t>x</t></r></is></c><c r="B5"/><c r="A5"><v>6</v></c><c r="A5" t="n"><v>7</v></c></row>
</sheetData>
<mergeCells count="1"><mergeCell ref="A1:B2"/></mergeCells>
<conditionalFormatting sqref="A1"><cfRule type="expression"/></conditionalFormatting>
<hyperlinks><hyperlink ref="A4" r:id="rId1"/><hyperlink ref="B4" location="Sheet2!A1"/></hyperlinks>
<drawing r:id="rId9"/>
</worksheet>"#,
            &rels,
        );
        assert!(sh.no_grid);
        assert_eq!(sh.frozen_rows, 2);
        assert_eq!(sh.default_row_ht, Some(18.0));
        assert_eq!(sh.default_col_w, Some(10.5));
        assert_eq!(sh.cols.len(), 2);
        assert_eq!((sh.cols[0].min, sh.cols[0].max, sh.cols[0].width), (0, 1, Some(12.7)));
        assert!(sh.cols[1].hidden);
        assert_eq!(sh.cols[1].xf, 3);
        assert!(sh.has_cf);
        assert_eq!(sh.drawing_rid.as_deref(), Some("rId9"));
        assert_eq!(sh.merges, vec![Merge { r0: 1, c0: 0, r1: 2, c1: 1 }]);
        assert_eq!(sh.links.len(), 1);
        assert_eq!(sh.links[0].url, "https://example.com/a?b=1&c=2");
        assert_eq!(sh.links[0].range, Merge { r0: 4, c0: 0, r1: 4, c1: 0 });

        assert_eq!(sh.rows.len(), 4);
        let r1 = sh.rows[0];
        assert_eq!((r1.r, r1.ht, r1.hidden, r1.xf), (1, 30.0, false, NO_XF));
        let c = sh.cells_of(&r1);
        assert_eq!(c.len(), 3);
        assert_eq!((c[0].col, c[0].xf, c[0].val), (0, 2, Val::Shared(5)));
        assert_eq!(c[1].val, Val::Num(3.5));
        assert_eq!(c[2].val, Val::Bool(true));

        let r3 = sh.rows[1];
        assert!(r3.hidden);
        assert_eq!(r3.xf, 4);
        let c = sh.cells_of(&r3);
        let Val::Text(o, l) = c[0].val else { panic!("{:?}", c[0]) };
        assert_eq!(sh.str_at(o, l), "x & y");
        let Val::Err(o, l) = c[1].val else { panic!() };
        assert_eq!(sh.str_at(o, l), "#DIV/0!");

        // never-calculated formula shows its text
        let c = sh.cells_of(&sh.rows[2]);
        let Val::Text(o, l) = c[0].val else { panic!("{:?}", c[0]) };
        assert_eq!(sh.str_at(o, l), "=SUM(A1:A3)");
        let Val::Text(o, l) = c[1].val else { panic!() };
        assert_eq!(sh.str_at(o, l), "inline");
        assert_eq!((c[2].col, c[2].xf, c[2].val), (2, 1, Val::Empty));
        assert_eq!(c[3].val, Val::Date(45000.0));

        // out-of-order cells are sorted; the empty unstyled B5 is dropped
        let c = sh.cells_of(&sh.rows[3]);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].col, 0);
        assert_eq!(c[0].val, Val::Num(7.0));
        assert_eq!(c[1].col, 4);
        let Val::Rich(i) = c[1].val else { panic!() };
        assert_eq!(sh.rich[i as usize][0].text, "B");
        assert_eq!(sh.rich[i as usize][0].class, "b");
    }

    #[test]
    fn shared_formula_without_text_stays_empty() {
        let sh = parse_xml(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="1"><c r="A1"><f t="shared" ref="A1:A2" si="0">B1*2</f></c><c r="B1"><f t="shared" si="0"/></c></row></sheetData></worksheet>"#,
            &Rels::new(),
        );
        let c = sh.cells_of(&sh.rows[0]);
        assert_eq!(c.len(), 1);
        let Val::Text(o, l) = c[0].val else { panic!() };
        assert_eq!(sh.str_at(o, l), "=B1*2");
    }

    #[test]
    fn caps_rows_and_columns() {
        let mut xml =
            String::from(r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#);
        xml.push_str(r#"<row r="1"><c r="IW1"><v>1</v></c><c r="A1"><v>2</v></c></row>"#);
        xml.push_str(
            r#"<row r="100001"><c r="A100001"><v>1</v></c></row><row r="100002"><c r="B100002"><v>1</v></c></row>"#,
        );
        xml.push_str("</sheetData></worksheet>");
        let sh = parse_xml(&xml, &Rels::new());
        assert_eq!(sh.rows.len(), 1);
        assert_eq!(sh.cells.len(), 1);
        assert_eq!(sh.over_rows, 100_002);
        assert_eq!(sh.over_cols, 257);
    }

    #[test]
    fn prefixed_namespace() {
        let sh = parse_xml(
            r#"<x:worksheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><x:sheetData><x:row r="2"><x:c r="B2" t="s"><x:v>1</x:v></x:c></x:row></x:sheetData></x:worksheet>"#,
            &Rels::new(),
        );
        assert_eq!(sh.rows[0].r, 2);
        assert_eq!(sh.cells[0].col, 1);
        assert_eq!(sh.cells[0].val, Val::Shared(1));
    }
}
