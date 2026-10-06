//! Excel number formats: parses a format code once, then formats numbers, dates and text with it
//! the way Excel displays them (zh-CN flavoured: built-in ids 27–36 and 50–58 are Chinese dates).

use crate::color::{self, Rgb};
use std::fmt::Write;

// ---- built-in formats ---------------------------------------------------------------------

/// The format code of a built-in `numFmtId` (zh-CN locale), `None` for ids Excel leaves open.
pub fn builtin(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "\"¥\"#,##0_);\\(\"¥\"#,##0\\)",
        6 => "\"¥\"#,##0_);[Red]\\(\"¥\"#,##0\\)",
        7 => "\"¥\"#,##0.00_);\\(\"¥\"#,##0.00\\)",
        8 => "\"¥\"#,##0.00_);[Red]\\(\"¥\"#,##0.00\\)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "yyyy/m/d",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "yyyy/m/d h:mm",
        27 => "yyyy\"年\"m\"月\"",
        28 => "m\"月\"d\"日\"",
        29 => "m\"月\"d\"日\"",
        30 => "m-d-yy",
        31 => "yyyy\"年\"m\"月\"d\"日\"",
        32 => "h\"时\"mm\"分\"",
        33 => "h\"时\"mm\"分\"ss\"秒\"",
        34 => "上午/下午h\"时\"mm\"分\"",
        35 => "上午/下午h\"时\"mm\"分\"ss\"秒\"",
        36 => "yyyy\"年\"m\"月\"",
        37 => "#,##0_);(#,##0)",
        38 => "#,##0_);[Red](#,##0)",
        39 => "#,##0.00_);(#,##0.00)",
        40 => "#,##0.00_);[Red](#,##0.00)",
        41 => "_(* #,##0_);_(* \\(#,##0\\);_(* \"-\"_);_(@_)",
        42 => "_(\"¥\"* #,##0_);_(\"¥\"* \\(#,##0\\);_(\"¥\"* \"-\"_);_(@_)",
        43 => "_(* #,##0.00_);_(* \\(#,##0.00\\);_(* \"-\"??_);_(@_)",
        44 => "_(\"¥\"* #,##0.00_);_(\"¥\"* \\(#,##0.00\\);_(\"¥\"* \"-\"??_);_(@_)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mm:ss.0",
        48 => "##0.0E+0",
        49 => "@",
        50 => "yyyy\"年\"m\"月\"",
        51 => "m\"月\"d\"日\"",
        52 => "yyyy\"年\"m\"月\"",
        53 => "m\"月\"d\"日\"",
        54 => "m\"月\"d\"日\"",
        55 => "上午/下午h\"时\"mm\"分\"",
        56 => "上午/下午h\"时\"mm\"分\"ss\"秒\"",
        57 => "yyyy\"年\"m\"月\"",
        58 => "m\"月\"d\"日\"",
        _ => return None,
    })
}

// ---- parsed form --------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Cond {
    cmp: Cmp,
    val: f64,
}

impl Cond {
    fn test(self, v: f64) -> bool {
        match self.cmp {
            Cmp::Lt => v < self.val,
            Cmp::Le => v <= self.val,
            Cmp::Gt => v > self.val,
            Cmp::Ge => v >= self.val,
            Cmp::Eq => v == self.val,
            Cmp::Ne => v != self.val,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Ampm {
    short: bool,
    lower: bool,
    cn: bool,
}

#[derive(Clone, PartialEq, Debug)]
enum Tok {
    Lit(String),
    /// `0`, `#` or `?`.
    Digit(u8),
    Point,
    Comma,
    Percent,
    /// `E+` (true) / `E-`.
    Exp(bool),
    Slash,
    /// `@`.
    Text,
    General,
    Year(u8),
    Month(u8),
    Min(u8),
    Day(u8),
    Hour(u8),
    Sec(u8),
    /// Fractional seconds digits (`.00`).
    Frac(u8),
    Ampm(Ampm),
    /// `[h]`, `[m]`, `[s]` (letter, width).
    Elapsed(u8, u8),
    /// `aaa` / `aaaa`: Chinese weekday.
    CnWeek(u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Number,
    Date,
    Text,
}

#[derive(Clone, Debug, Default)]
struct FracInfo {
    /// Index of the `/` token.
    slash: usize,
    /// Index of the first numerator placeholder.
    num_start: usize,
    num_ph: usize,
    den_ph: usize,
    /// A literal denominator (`# ?/8`).
    fixed_den: Option<u32>,
}

#[derive(Clone, Debug, Default)]
struct NumInfo {
    point: Option<usize>,
    exp: Option<usize>,
    n_int: usize,
    n_frac: usize,
    int_has_hash: bool,
    thousands: bool,
    scale: i32,
    pct: i32,
    exp_digits: usize,
    frac: Option<FracInfo>,
}

#[derive(Clone, Debug)]
struct Section {
    cond: Option<Cond>,
    color: Option<Rgb>,
    toks: Vec<Tok>,
    kind: Kind,
    num: NumInfo,
    hour12: bool,
    frac_digits: u8,
    has_sec: bool,
    zh: bool,
}

/// A parsed format code.
#[derive(Clone, Debug)]
pub struct Format {
    sections: Vec<Section>,
    general: bool,
}

impl Format {
    /// `General`.
    pub fn general() -> Format {
        parse("General")
    }

    /// Whether the code is plain `General` (the fast path).
    pub fn is_general(&self) -> bool {
        self.general
    }

    /// Whether numbers are shown as dates / times by the first section.
    pub fn is_date(&self) -> bool {
        self.sections.first().is_some_and(|s| s.kind == Kind::Date)
    }

    /// Whether the format has a section that decorates text values (`@`, or a 4th section).
    pub fn has_text_section(&self) -> bool {
        self.text_section().is_some()
    }

    fn text_section(&self) -> Option<&Section> {
        if self.sections.len() >= 4 {
            return self.sections.get(3);
        }
        self.sections.iter().find(|s| s.kind == Kind::Text)
    }

    fn pick(&self, v: f64) -> (usize, bool) {
        let n = self.sections.len().min(3);
        for i in 0..n {
            let s = &self.sections[i];
            let ok = match s.cond {
                Some(c) => c.test(v),
                None => {
                    if i + 1 == n {
                        true
                    } else if i == 0 {
                        if n == 2 {
                            v >= 0.0
                        } else {
                            v > 0.0
                        }
                    } else {
                        v < 0.0
                    }
                }
            };
            if ok {
                let neg_sec = i >= 1 || s.cond.is_some_and(|c| matches!(c.cmp, Cmp::Lt | Cmp::Le) && c.val <= 0.0);
                return (i, v < 0.0 && !neg_sec);
            }
        }
        (n.saturating_sub(1), v < 0.0 && n <= 1)
    }

    /// Appends the display of number `v` to `out`; returns the color the format asks for.
    pub fn number(&self, v: f64, date1904: bool, out: &mut String) -> Option<Rgb> {
        if !v.is_finite() {
            out.push_str("#NUM!");
            return None;
        }
        if self.general {
            general(v, out);
            return None;
        }
        let (idx, minus) = self.pick(v);
        let sec = &self.sections[idx];
        match sec.kind {
            Kind::Date => {
                if !write_date(sec, v, date1904, out) {
                    out.push_str("########");
                }
            }
            Kind::Text => general(v, out),
            Kind::Number => {
                if minus {
                    out.push('-');
                }
                write_number(sec, v.abs(), out);
            }
        }
        sec.color
    }

    /// Appends the display of text `s`.
    pub fn text(&self, s: &str, out: &mut String) -> Option<Rgb> {
        let Some(sec) = self.text_section() else {
            out.push_str(s);
            return None;
        };
        for t in &sec.toks {
            match t {
                Tok::Text | Tok::General => out.push_str(s),
                Tok::Lit(l) => out.push_str(l),
                Tok::Percent => out.push('%'),
                Tok::Point => out.push('.'),
                Tok::Slash => out.push('/'),
                _ => {}
            }
        }
        sec.color
    }
}

// ---- parsing ------------------------------------------------------------------------------

/// Parses a format code.
pub fn parse(code: &str) -> Format {
    let general = code.trim().eq_ignore_ascii_case("general");
    let mut sections: Vec<Section> = split_sections(code).iter().map(|s| parse_section(s)).collect();
    if sections.is_empty() {
        sections.push(parse_section("General"));
    }
    Format { sections, general }
}

fn split_sections(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = code.chars().peekable();
    let mut in_q = false;
    let mut in_b = false;
    while let Some(c) = chars.next() {
        match c {
            '"' if !in_b => {
                in_q = !in_q;
                cur.push(c);
            }
            '\\' | '_' | '*' if !in_q && !in_b => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            '[' if !in_q => {
                in_b = true;
                cur.push(c);
            }
            ']' if !in_q => {
                in_b = false;
                cur.push(c);
            }
            ';' if !in_q && !in_b => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn push_lit(toks: &mut Vec<Tok>, s: &str) {
    if s.is_empty() {
        return;
    }
    if let Some(Tok::Lit(l)) = toks.last_mut() {
        l.push_str(s);
    } else {
        toks.push(Tok::Lit(s.to_string()));
    }
}

fn starts_ci(chars: &[char], i: usize, pat: &str) -> bool {
    let mut k = i;
    for p in pat.chars() {
        match chars.get(k) {
            Some(c) if c.to_ascii_lowercase() == p => k += 1,
            _ => return false,
        }
    }
    true
}

fn named_color(name: &str) -> Option<Rgb> {
    let n = name.trim().to_ascii_lowercase();
    Some(match n.as_str() {
        "black" => Rgb(0, 0, 0),
        "blue" => Rgb(0, 0, 255),
        "cyan" => Rgb(0, 255, 255),
        "green" => Rgb(0, 255, 0),
        "magenta" => Rgb(255, 0, 255),
        "red" => Rgb(255, 0, 0),
        "white" => Rgb(255, 255, 255),
        "yellow" => Rgb(255, 255, 0),
        _ => {
            let k: usize = n.strip_prefix("color")?.trim().parse().ok()?;
            if !(1..=56).contains(&k) {
                return None;
            }
            return color::indexed(k + 7, &[]);
        }
    })
}

fn parse_cond(s: &str) -> Option<Cond> {
    let (cmp, rest) = if let Some(r) = s.strip_prefix("<=") {
        (Cmp::Le, r)
    } else if let Some(r) = s.strip_prefix(">=") {
        (Cmp::Ge, r)
    } else if let Some(r) = s.strip_prefix("<>") {
        (Cmp::Ne, r)
    } else if let Some(r) = s.strip_prefix('<') {
        (Cmp::Lt, r)
    } else if let Some(r) = s.strip_prefix('>') {
        (Cmp::Gt, r)
    } else if let Some(r) = s.strip_prefix('=') {
        (Cmp::Eq, r)
    } else {
        return None;
    };
    Some(Cond { cmp, val: rest.trim().parse().ok()? })
}

fn parse_section(src: &str) -> Section {
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut toks: Vec<Tok> = Vec::new();
    let mut cond = None;
    let mut color = None;
    let mut zh = false;
    let mut i = 0;
    while i < n {
        let c = chars[i];
        match c {
            '"' => {
                let mut j = i + 1;
                let mut s = String::new();
                while j < n && chars[j] != '"' {
                    s.push(chars[j]);
                    j += 1;
                }
                push_lit(&mut toks, &s);
                i = j + 1;
            }
            '\\' => {
                if let Some(&ch) = chars.get(i + 1) {
                    push_lit(&mut toks, &ch.to_string());
                }
                i += 2;
            }
            '_' => {
                push_lit(&mut toks, " ");
                i += 2;
            }
            '*' => i += 2,
            '[' => {
                let mut j = i + 1;
                let mut s = String::new();
                while j < n && chars[j] != ']' {
                    s.push(chars[j]);
                    j += 1;
                }
                i = j + 1;
                let t = s.trim();
                if let Some(rest) = t.strip_prefix('$') {
                    let (cur, lcid) = match rest.rfind('-') {
                        Some(k) => (&rest[..k], &rest[k + 1..]),
                        None => (rest, ""),
                    };
                    push_lit(&mut toks, cur);
                    if let Ok(l) = u32::from_str_radix(lcid, 16) {
                        if l & 0xFF == 0x04 {
                            zh = true;
                        }
                    }
                } else if let Some(c) = parse_cond(t) {
                    cond = Some(c);
                } else if let Some(c) = named_color(t) {
                    color = Some(c);
                } else {
                    let lower = t.to_ascii_lowercase();
                    let b = lower.as_bytes();
                    if !b.is_empty()
                        && b.len() <= 2
                        && b.iter().all(|x| *x == b[0])
                        && matches!(b[0], b'h' | b'm' | b's')
                    {
                        toks.push(Tok::Elapsed(b[0], b.len() as u8));
                    }
                }
            }
            '0' | '#' | '?' => {
                toks.push(Tok::Digit(c as u8));
                i += 1;
            }
            '.' => {
                let after_sec = matches!(toks.last(), Some(Tok::Sec(_)) | Some(Tok::Elapsed(b's', _)));
                if after_sec && chars.get(i + 1) == Some(&'0') {
                    let mut j = i + 1;
                    while j < n && chars[j] == '0' {
                        j += 1;
                    }
                    toks.push(Tok::Frac((j - i - 1).min(3) as u8));
                    i = j;
                } else {
                    toks.push(Tok::Point);
                    i += 1;
                }
            }
            ',' => {
                toks.push(Tok::Comma);
                i += 1;
            }
            '%' => {
                toks.push(Tok::Percent);
                i += 1;
            }
            '/' => {
                toks.push(Tok::Slash);
                i += 1;
            }
            '@' => {
                toks.push(Tok::Text);
                i += 1;
            }
            'E' | 'e' if matches!(chars.get(i + 1), Some('+') | Some('-')) => {
                toks.push(Tok::Exp(chars[i + 1] == '+'));
                i += 2;
            }
            'G' | 'g' if starts_ci(&chars, i, "general") => {
                toks.push(Tok::General);
                i += 7;
            }
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' => {
                let lc = c.to_ascii_lowercase();
                let mut j = i;
                while j < n && chars[j].to_ascii_lowercase() == lc {
                    j += 1;
                }
                let len = (j - i) as u8;
                toks.push(match lc {
                    'y' => Tok::Year(if len <= 2 { 2 } else { 4 }),
                    'm' => Tok::Month(len.min(5)),
                    'd' => Tok::Day(len.min(4)),
                    'h' => Tok::Hour(len.min(2)),
                    _ => Tok::Sec(len.min(2)),
                });
                i = j;
            }
            'a' | 'A' => {
                if starts_ci(&chars, i, "am/pm") {
                    let lower = chars[i] == 'a' && chars[i + 1] == 'm';
                    toks.push(Tok::Ampm(Ampm { short: false, lower, cn: false }));
                    i += 5;
                } else if starts_ci(&chars, i, "a/p") {
                    toks.push(Tok::Ampm(Ampm { short: true, lower: chars[i] == 'a', cn: false }));
                    i += 3;
                } else {
                    let mut j = i;
                    while j < n && chars[j].to_ascii_lowercase() == 'a' {
                        j += 1;
                    }
                    if j - i >= 3 {
                        toks.push(Tok::CnWeek((j - i).min(4) as u8));
                    } else {
                        push_lit(&mut toks, &chars[i..j].iter().collect::<String>());
                    }
                    i = j;
                }
            }
            '上' if chars[i..].starts_with(&['上', '午', '/', '下', '午']) => {
                toks.push(Tok::Ampm(Ampm { short: false, lower: false, cn: true }));
                i += 5;
            }
            _ => {
                push_lit(&mut toks, &c.to_string());
                i += 1;
            }
        }
    }
    let mut sec = Section {
        cond,
        color,
        toks,
        kind: Kind::Number,
        num: NumInfo::default(),
        hour12: false,
        frac_digits: 0,
        has_sec: false,
        zh,
    };
    analyze(&mut sec);
    sec
}

fn is_date_tok(t: &Tok) -> bool {
    matches!(
        t,
        Tok::Year(_)
            | Tok::Month(_)
            | Tok::Min(_)
            | Tok::Day(_)
            | Tok::Hour(_)
            | Tok::Sec(_)
            | Tok::Ampm(_)
            | Tok::Elapsed(..)
            | Tok::CnWeek(_)
    )
}

fn analyze(sec: &mut Section) {
    if sec.toks.iter().any(is_date_tok) {
        sec.kind = Kind::Date;
        // `m` is minutes right after hours or right before seconds.
        let n = sec.toks.len();
        for i in 0..n {
            let Tok::Month(w) = sec.toks[i] else { continue };
            if w > 2 {
                continue;
            }
            let prev = sec.toks[..i].iter().rev().find(|t| !matches!(t, Tok::Lit(_)));
            let next = sec.toks[i + 1..].iter().find(|t| !matches!(t, Tok::Lit(_)));
            if matches!(prev, Some(Tok::Hour(_)) | Some(Tok::Elapsed(b'h', _)))
                || matches!(next, Some(Tok::Sec(_)) | Some(Tok::Elapsed(b's', _)))
            {
                sec.toks[i] = Tok::Min(w);
            }
        }
        sec.hour12 = sec.toks.iter().any(|t| matches!(t, Tok::Ampm(_)));
        sec.frac_digits =
            sec.toks.iter().filter_map(|t| if let Tok::Frac(d) = t { Some(*d) } else { None }).max().unwrap_or(0);
        sec.has_sec = sec.toks.iter().any(|t| matches!(t, Tok::Sec(_) | Tok::Elapsed(b's', _)));
        return;
    }
    let has_digit = sec.toks.iter().any(|t| matches!(t, Tok::Digit(_)));
    if !has_digit && sec.toks.iter().any(|t| matches!(t, Tok::Text)) {
        sec.kind = Kind::Text;
        return;
    }
    // Commas: between digits = thousands separator, after the last digit = ÷1000 each.
    let first = sec.toks.iter().position(|t| matches!(t, Tok::Digit(_)));
    let last = sec.toks.iter().rposition(|t| matches!(t, Tok::Digit(_)));
    let mut info = NumInfo::default();
    let mut toks: Vec<Tok> = Vec::with_capacity(sec.toks.len());
    for (i, t) in std::mem::take(&mut sec.toks).into_iter().enumerate() {
        match (&t, first, last) {
            (Tok::Comma, Some(f), Some(l)) if i > f && i < l => info.thousands = true,
            (Tok::Comma, Some(f), Some(l)) if i > l && i > f => info.scale += 1,
            (Tok::Comma, ..) => push_lit(&mut toks, ","),
            (Tok::Lit(s), ..) => push_lit(&mut toks, s),
            _ => toks.push(t),
        }
    }
    info.pct = toks.iter().filter(|t| matches!(t, Tok::Percent)).count() as i32;
    info.point = toks.iter().position(|t| matches!(t, Tok::Point));
    info.exp = toks.iter().position(|t| matches!(t, Tok::Exp(_)));
    let end_int = [info.point, info.exp].into_iter().flatten().min().unwrap_or(toks.len());
    let slash = toks.iter().position(|t| matches!(t, Tok::Slash));
    // Fraction format `# ?/?`: placeholders on both sides of a slash, no decimal point.
    if let (Some(s), None, None) = (slash, info.point, info.exp) {
        let num_ph = toks[..s].iter().rev().take_while(|t| matches!(t, Tok::Digit(_))).count();
        let den_ph = toks[s + 1..].iter().take_while(|t| matches!(t, Tok::Digit(_))).count();
        let mut fixed = None;
        if den_ph == 0 {
            if let Some(Tok::Lit(l)) = toks.get_mut(s + 1) {
                let digits: String = l.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(d) = digits.parse::<u32>() {
                    if d > 0 {
                        fixed = Some(d);
                        *l = l[digits.len()..].to_string();
                    }
                }
            }
        }
        if num_ph > 0 && (den_ph > 0 || fixed.is_some()) {
            let num_start = s - num_ph;
            info.n_int = toks[..num_start].iter().filter(|t| matches!(t, Tok::Digit(_))).count();
            info.int_has_hash = toks[..num_start].iter().any(|t| matches!(t, Tok::Digit(b'#')));
            info.frac = Some(FracInfo { slash: s, num_start, num_ph, den_ph, fixed_den: fixed });
            sec.num = info;
            sec.toks = toks;
            return;
        }
    }
    let digit_idx: Vec<usize> =
        toks.iter().enumerate().filter(|(_, t)| matches!(t, Tok::Digit(_))).map(|(i, _)| i).collect();
    for &i in &digit_idx {
        let Tok::Digit(k) = toks[i] else { continue };
        if info.exp.is_some_and(|e| i > e) {
            info.exp_digits += 1;
        } else if info.point.is_some_and(|p| i > p) {
            info.n_frac += 1;
        } else if i < end_int {
            info.n_int += 1;
            if k == b'#' {
                info.int_has_hash = true;
            }
        }
    }
    sec.num = info;
    sec.toks = toks;
}

// ---- digits -------------------------------------------------------------------------------

struct StackBuf {
    buf: [u8; 48],
    len: usize,
}

impl Write for StackBuf {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let b = s.as_bytes();
        if self.len + b.len() > self.buf.len() {
            return Err(std::fmt::Error);
        }
        self.buf[self.len..self.len + b.len()].copy_from_slice(b);
        self.len += b.len();
        Ok(())
    }
}

/// The 15 significant decimal digits Excel works with: value = d0.d1d2… × 10^e.
struct Sig {
    d: [u8; 15],
    e: i32,
}

fn sig15(x: f64) -> Sig {
    let mut buf = StackBuf { buf: [0; 48], len: 0 };
    let _ = write!(buf, "{:.14e}", x);
    let s = std::str::from_utf8(&buf.buf[..buf.len]).unwrap_or("0e0");
    let (m, ex) = s.split_once('e').unwrap_or((s, "0"));
    let mut d = [0u8; 15];
    let mut k = 0;
    for b in m.bytes() {
        if b.is_ascii_digit() && k < 15 {
            d[k] = b - b'0';
            k += 1;
        }
    }
    Sig { d, e: ex.parse().unwrap_or(0) }
}

fn digit_at(sig: &Sig, s: i32) -> u8 {
    if s < 0 || s >= 15 {
        0
    } else {
        sig.d[s as usize]
    }
}

/// Rounds (half up, on the 15-digit decimal value) to `n_frac` decimals, treating the number as
/// d0.d1… × 10^e. Returns (integer digits, fraction digits).
fn dec_round(sig: &Sig, e: i32, n_frac: usize) -> (String, String) {
    let ni = e + 1;
    let n_int = ni.max(0) as usize;
    let mut ints: Vec<u8> = (0..n_int).map(|i| digit_at(sig, i as i32)).collect();
    let mut fracs: Vec<u8> = (0..n_frac).map(|j| digit_at(sig, ni + j as i32)).collect();
    if digit_at(sig, ni + n_frac as i32) >= 5 {
        let mut carry = true;
        for d in fracs.iter_mut().rev().chain(ints.iter_mut().rev()) {
            if !carry {
                break;
            }
            if *d == 9 {
                *d = 0;
            } else {
                *d += 1;
                carry = false;
            }
        }
        if carry {
            ints.insert(0, 1);
        }
    }
    let mut i: String = ints.iter().map(|d| (b'0' + d) as char).collect();
    if i.is_empty() {
        i.push('0');
    }
    let f: String = fracs.iter().map(|d| (b'0' + d) as char).collect();
    (i, f)
}

/// Excel's General: at most 11 characters, scientific beyond that.
pub fn general(v: f64, out: &mut String) {
    if v == 0.0 {
        out.push('0');
        return;
    }
    if !v.is_finite() {
        out.push_str("#NUM!");
        return;
    }
    let neg = v < 0.0;
    let a = v.abs();
    if a.fract() == 0.0 && a < 1e11 {
        if neg {
            out.push('-');
        }
        let _ = write!(out, "{}", a as u64);
        return;
    }
    let sig = sig15(a);
    let sign = neg as i32;
    if (-4..11).contains(&sig.e) {
        let n_int = (sig.e + 1).max(1);
        let dec = (10 - sign - n_int).max(0) as usize;
        let (ip, fp) = dec_round(&sig, sig.e, dec);
        if ip.len() <= 11 {
            if neg {
                out.push('-');
            }
            out.push_str(&ip);
            let fp = fp.trim_end_matches('0');
            if !fp.is_empty() {
                out.push('.');
                out.push_str(fp);
            }
            return;
        }
    }
    // Scientific.
    let mut e = sig.e;
    let exp_len = if e.abs() >= 100 { 5 } else { 4 };
    let dec = (11 - sign - exp_len - 2).max(0) as usize;
    let (mut ip, mut fp) = dec_round(&sig, 0, dec);
    if ip.len() > 1 {
        e += 1;
        ip = "1".into();
        fp = "0".repeat(dec);
    }
    if neg {
        out.push('-');
    }
    out.push_str(&ip);
    let fp = fp.trim_end_matches('0');
    if !fp.is_empty() {
        out.push('.');
        out.push_str(fp);
    }
    let _ = write!(out, "E{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
}

// ---- numbers ------------------------------------------------------------------------------

fn write_number(sec: &Section, x: f64, out: &mut String) {
    let info = &sec.num;
    let mut v = x;
    if info.pct > 0 {
        v *= 10f64.powi(2 * info.pct);
    }
    if info.scale > 0 {
        v /= 1000f64.powi(info.scale);
    }
    if let Some(f) = &info.frac {
        write_fraction(sec, f, v, out);
        return;
    }
    let (ip, fp, exp) = if info.exp.is_some() {
        let (i, f, e) = sci_parts(v, info);
        (i, f, Some(e))
    } else {
        let sig = sig15(v);
        let (i, f) = dec_round(&sig, sig.e, info.n_frac);
        (i, f, None)
    };
    let ds: &str = if ip == "0" { "" } else { &ip };
    let last_sig = fp.bytes().rposition(|b| b != b'0');
    let mut int_seen = 0usize;
    let mut frac_seen = 0usize;
    let mut head_done = false;
    let emit_head = |out: &mut String, from: usize| {
        // Digits beyond the placeholders, leftmost first.
        let len = ds.len();
        let mut q = len;
        while q > from {
            q -= 1;
            out.push(ds.as_bytes()[len - 1 - q] as char);
            if info.thousands && q > 0 && q % 3 == 0 {
                out.push(',');
            }
        }
    };
    for (i, t) in sec.toks.iter().enumerate() {
        match t {
            Tok::Digit(k) => {
                if info.exp.is_some_and(|e| i > e) {
                    continue;
                }
                if info.point.is_some_and(|p| i > p) {
                    let j = frac_seen;
                    frac_seen += 1;
                    let digit = fp.as_bytes().get(j).copied().unwrap_or(b'0');
                    if last_sig.is_some_and(|l| j <= l) {
                        out.push(digit as char);
                    } else {
                        match k {
                            b'0' => out.push('0'),
                            b'?' => out.push(' '),
                            _ => {}
                        }
                    }
                } else {
                    if !head_done {
                        head_done = true;
                        emit_head(out, info.n_int);
                    }
                    let p = info.n_int - 1 - int_seen;
                    int_seen += 1;
                    let mut digit = false;
                    if p < ds.len() {
                        out.push(ds.as_bytes()[ds.len() - 1 - p] as char);
                        digit = true;
                    } else {
                        match k {
                            b'0' => {
                                out.push('0');
                                digit = true;
                            }
                            b'?' => out.push(' '),
                            _ => {}
                        }
                    }
                    if digit && info.thousands && p > 0 && p % 3 == 0 {
                        out.push(',');
                    }
                }
            }
            Tok::Point => {
                if info.n_int == 0 && !head_done && info.exp.is_none() {
                    head_done = true;
                    emit_head(out, 0);
                }
                out.push('.');
            }
            Tok::Exp(plus) => {
                if info.n_int == 0 && !head_done {
                    head_done = true;
                    emit_head(out, 0);
                }
                let e = exp.unwrap_or(0);
                out.push('E');
                if e < 0 {
                    out.push('-');
                } else if *plus {
                    out.push('+');
                }
                let _ = write!(out, "{:0w$}", e.abs(), w = info.exp_digits.max(1));
            }
            Tok::Percent => out.push('%'),
            Tok::Slash => out.push('/'),
            Tok::Lit(l) => out.push_str(l),
            Tok::General => general(x, out),
            _ => {}
        }
    }
}

/// Mantissa digits and exponent for `0.00E+00` style formats.
fn sci_parts(v: f64, info: &NumInfo) -> (String, String, i32) {
    let sig = sig15(v);
    if sig.d[0] == 0 {
        let (i, f) = dec_round(&sig, 0, info.n_frac);
        return (i, f, 0);
    }
    let n_int = info.n_int.max(1) as i32;
    let eng = n_int > 1 && info.int_has_hash;
    let step = if eng { n_int } else { 1 };
    let mut exp = if eng { sig.e.div_euclid(n_int) * n_int } else { sig.e - (n_int - 1) };
    loop {
        let (i, f) = dec_round(&sig, sig.e - exp, info.n_frac);
        if i.len() as i32 > n_int && i != "0" {
            exp += step;
            continue;
        }
        return (i, f, exp);
    }
}

fn write_fraction(sec: &Section, f: &FracInfo, v: f64, out: &mut String) {
    let info = &sec.num;
    let has_int = info.n_int > 0;
    let whole_f = v.trunc();
    let frac = if has_int { v - whole_f } else { v };
    let max_den = match f.fixed_den {
        Some(d) => d,
        None => 10u32.pow(f.den_ph.min(4) as u32) - 1,
    };
    let (mut p, mut q) = (0u64, 1u64);
    let mut best = f64::MAX;
    let range = match f.fixed_den {
        Some(d) => d..=d,
        None => 1..=max_den.max(1),
    };
    for den in range {
        let num = (frac * den as f64).round();
        let err = (frac - num / den as f64).abs();
        if err < best - 1e-12 {
            best = err;
            p = num as u64;
            q = den as u64;
            if err == 0.0 {
                break;
            }
        }
    }
    let mut whole = whole_f as u64;
    if has_int && p == q {
        whole += 1;
        p = 0;
    }
    let ds = whole.to_string();
    let ds: &str = if whole == 0 { "" } else { &ds };
    let show_frac = p != 0 || !has_int;
    let num_s = p.to_string();
    let den_s = q.to_string();
    let mut int_seen = 0usize;
    let mut i = 0;
    let toks = &sec.toks;
    while i < toks.len() {
        if i == f.num_start {
            // numerator + slash + denominator
            if show_frac {
                for _ in num_s.len()..f.num_ph {
                    out.push(' ');
                }
                out.push_str(&num_s);
                out.push('/');
                if f.fixed_den.is_some() {
                    out.push_str(&den_s);
                } else {
                    out.push_str(&den_s);
                    for _ in den_s.len()..f.den_ph {
                        out.push(' ');
                    }
                }
            } else {
                let w = f.num_ph + 1 + if f.fixed_den.is_some() { den_s.len() } else { f.den_ph };
                // `#` placeholders collapse to nothing, `?` to blanks.
                if toks[f.num_start..f.slash].iter().any(|t| matches!(t, Tok::Digit(b'?'))) {
                    for _ in 0..w {
                        out.push(' ');
                    }
                }
            }
            i = f.slash + 1 + f.den_ph;
            continue;
        }
        match &toks[i] {
            Tok::Digit(k) if i < f.num_start => {
                let pos = info.n_int - 1 - int_seen;
                int_seen += 1;
                if int_seen == 1 {
                    // Digits beyond the placeholders first.
                    let len = ds.len();
                    let mut q2 = len;
                    while q2 > info.n_int {
                        q2 -= 1;
                        out.push(ds.as_bytes()[len - 1 - q2] as char);
                    }
                }
                if pos < ds.len() {
                    out.push(ds.as_bytes()[ds.len() - 1 - pos] as char);
                } else {
                    match k {
                        b'0' => out.push('0'),
                        b'?' => out.push(' '),
                        _ => {}
                    }
                }
            }
            Tok::Lit(l) => out.push_str(l),
            Tok::Percent => out.push('%'),
            _ => {}
        }
        i += 1;
    }
}

// ---- dates --------------------------------------------------------------------------------

const EN_MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const EN_DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const CN_NUM: [&str; 12] = ["一", "二", "三", "四", "五", "六", "七", "八", "九", "十", "十一", "十二"];
const CN_WEEK: [&str; 7] = ["日", "一", "二", "三", "四", "五", "六"];

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The serial number of a calendar date in the 1900 system (with Excel's phantom Feb 29, 1900).
pub fn serial_from_ymd(y: i64, m: i64, d: i64, date1904: bool) -> f64 {
    let unix = days_from_civil(y, m, d);
    if date1904 {
        return (unix - days_from_civil(1904, 1, 1)) as f64;
    }
    let s = unix + 25569;
    (if s < 61 { s - 1 } else { s }) as f64
}

struct Dt {
    y: i64,
    m: u32,
    d: u32,
    wd: usize,
    /// Milliseconds since midnight plus whole days in `days`.
    tms: i64,
}

fn to_dt(serial: f64, date1904: bool, frac_digits: u8) -> Option<Dt> {
    if serial < 0.0 || serial >= 2_958_466.0 {
        return None;
    }
    let gran: i64 = match frac_digits {
        0 => 1000,
        1 => 100,
        2 => 10,
        _ => 1,
    };
    let total = (serial * 86_400_000.0).round() as i64;
    let total = (total + gran / 2) / gran * gran;
    let days = total.div_euclid(86_400_000);
    let tms = total.rem_euclid(86_400_000);
    let (y, m, d, wd);
    if date1904 {
        let unix = days + days_from_civil(1904, 1, 1);
        let c = civil_from_days(unix);
        (y, m, d) = c;
        wd = (unix + 4).rem_euclid(7) as usize;
    } else {
        wd = (days - 1).rem_euclid(7) as usize;
        if days == 0 {
            (y, m, d) = (1900, 1, 0);
        } else if days == 60 {
            (y, m, d) = (1900, 2, 29);
        } else {
            let unix = if days < 60 { days - 25568 } else { days - 25569 };
            (y, m, d) = civil_from_days(unix);
        }
    }
    Some(Dt { y, m, d, wd, tms })
}

fn write_date(sec: &Section, serial: f64, date1904: bool, out: &mut String) -> bool {
    let Some(dt) = to_dt(serial, date1904, sec.frac_digits) else { return false };
    let tms = dt.tms;
    let total_ms = (serial * 86_400_000.0).round() as i64;
    let gran: i64 = match sec.frac_digits {
        0 => 1000,
        1 => 100,
        2 => 10,
        _ => 1,
    };
    let total_ms = (total_ms + gran / 2) / gran * gran;
    let hour = (tms / 3_600_000) as u32;
    let minute = ((tms / 60_000) % 60) as u32;
    let second = ((tms / 1000) % 60) as u32;
    let ms = (tms % 1000) as u32;
    for t in &sec.toks {
        match t {
            Tok::Lit(l) => out.push_str(l),
            Tok::Year(2) => {
                let _ = write!(out, "{:02}", dt.y.rem_euclid(100));
            }
            Tok::Year(_) => {
                let _ = write!(out, "{:04}", dt.y);
            }
            Tok::Month(w) => match w {
                1 => {
                    let _ = write!(out, "{}", dt.m);
                }
                2 => {
                    let _ = write!(out, "{:02}", dt.m);
                }
                3 => {
                    if sec.zh {
                        let _ = write!(out, "{}月", dt.m);
                    } else {
                        out.push_str(&EN_MONTHS[dt.m.clamp(1, 12) as usize - 1][..3]);
                    }
                }
                4 => {
                    if sec.zh {
                        let _ = write!(out, "{}月", CN_NUM[dt.m.clamp(1, 12) as usize - 1]);
                    } else {
                        out.push_str(EN_MONTHS[dt.m.clamp(1, 12) as usize - 1]);
                    }
                }
                _ => out.push_str(&EN_MONTHS[dt.m.clamp(1, 12) as usize - 1][..1]),
            },
            Tok::Day(w) => match w {
                1 => {
                    let _ = write!(out, "{}", dt.d);
                }
                2 => {
                    let _ = write!(out, "{:02}", dt.d);
                }
                3 => {
                    if sec.zh {
                        let _ = write!(out, "周{}", CN_WEEK[dt.wd]);
                    } else {
                        out.push_str(&EN_DAYS[dt.wd][..3]);
                    }
                }
                _ => {
                    if sec.zh {
                        let _ = write!(out, "星期{}", CN_WEEK[dt.wd]);
                    } else {
                        out.push_str(EN_DAYS[dt.wd]);
                    }
                }
            },
            Tok::CnWeek(3) => {
                // Excel zh-CN: `aaa` is the bare weekday character (三), `aaaa` 星期三.
                out.push_str(CN_WEEK[dt.wd]);
            }
            Tok::CnWeek(_) => {
                let _ = write!(out, "星期{}", CN_WEEK[dt.wd]);
            }
            Tok::Hour(w) => {
                let h = if sec.hour12 {
                    if hour % 12 == 0 {
                        12
                    } else {
                        hour % 12
                    }
                } else {
                    hour
                };
                if *w >= 2 {
                    let _ = write!(out, "{:02}", h);
                } else {
                    let _ = write!(out, "{}", h);
                }
            }
            Tok::Min(w) => {
                if *w >= 2 {
                    let _ = write!(out, "{:02}", minute);
                } else {
                    let _ = write!(out, "{}", minute);
                }
            }
            Tok::Sec(w) => {
                if *w >= 2 {
                    let _ = write!(out, "{:02}", second);
                } else {
                    let _ = write!(out, "{}", second);
                }
            }
            Tok::Frac(d) => {
                let s = format!("{:03}", ms);
                out.push('.');
                out.push_str(&s[..(*d as usize).min(3)]);
            }
            Tok::Ampm(a) => {
                let pm = hour >= 12;
                let s = if a.cn {
                    if pm {
                        "下午"
                    } else {
                        "上午"
                    }
                } else if a.short {
                    if pm {
                        "P"
                    } else {
                        "A"
                    }
                } else if pm {
                    "PM"
                } else {
                    "AM"
                };
                if a.lower {
                    out.push_str(&s.to_ascii_lowercase());
                } else {
                    out.push_str(s);
                }
            }
            Tok::Elapsed(l, w) => {
                let v = match l {
                    b'h' => total_ms / 3_600_000,
                    b'm' => total_ms / 60_000,
                    _ => total_ms / 1000,
                };
                let _ = write!(out, "{:0w$}", v, w = *w as usize);
            }
            Tok::Percent => out.push('%'),
            Tok::Point => out.push('.'),
            Tok::Slash => out.push('/'),
            Tok::Comma => out.push(','),
            _ => {}
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(code: &str, v: f64) -> String {
        let mut s = String::new();
        parse(code).number(v, false, &mut s);
        s
    }

    fn t(code: &str, v: &str) -> String {
        let mut s = String::new();
        parse(code).text(v, &mut s);
        s
    }

    fn ser(y: i64, m: i64, d: i64) -> f64 {
        serial_from_ymd(y, m, d, false)
    }

    #[test]
    fn general_numbers() {
        assert_eq!(n("General", 0.0), "0");
        assert_eq!(n("General", 42.0), "42");
        assert_eq!(n("General", -42.0), "-42");
        assert_eq!(n("General", 3.14159), "3.14159");
        assert_eq!(n("General", 0.1 + 0.2), "0.3");
        assert_eq!(n("General", 123456.789012345), "123456.789");
        assert_eq!(n("General", 1234567.891012), "1234567.891");
        assert_eq!(n("General", 0.123456789012), "0.123456789");
        assert_eq!(n("General", -0.123456789012), "-0.12345679");
        assert_eq!(n("General", 12345678901.0), "12345678901");
        assert_eq!(n("General", 123456789012.0), "1.23457E+11");
        assert_eq!(n("General", -123456789012.0), "-1.2346E+11");
        assert_eq!(n("General", 1e20), "1E+20");
        assert_eq!(n("General", 0.0001), "0.0001");
        assert_eq!(n("General", 0.00001), "1E-05");
        assert_eq!(n("General", 0.000012345), "1.2345E-05");
        assert_eq!(n("General", 1e-10), "1E-10");
        assert_eq!(n("General", 1.5e300), "1.5E+300");
        assert_eq!(n("General", 99999999999.5), "1E+11");
    }

    #[test]
    fn plain_digits() {
        assert_eq!(n("0", 3.5), "4");
        assert_eq!(n("0", 2.5), "3");
        assert_eq!(n("0", 0.4), "0");
        assert_eq!(n("0.00", 3.14159), "3.14");
        assert_eq!(n("0.00", 2.675), "2.68");
        assert_eq!(n("0.00", 1.005), "1.01");
        assert_eq!(n("0.00", 0.5), "0.50");
        assert_eq!(n("0.00", -0.5), "-0.50");
        assert_eq!(n("0.0", 9.96), "10.0");
        assert_eq!(n("00000", 42.0), "00042");
        assert_eq!(n("000", 12345.0), "12345");
        assert_eq!(n("#", 0.0), "");
        assert_eq!(n("#.##", 0.5), ".5");
        assert_eq!(n("#.##", 5.0), "5.");
        assert_eq!(n("0.##", 1.5), "1.5");
        assert_eq!(n("0.0?", 1.5), "1.5 ");
        assert_eq!(n("???", 5.0), "  5");
        assert_eq!(n(".00", 1.5), "1.50");
    }

    #[test]
    fn thousands_and_scale() {
        assert_eq!(n("#,##0", 1234567.0), "1,234,567");
        assert_eq!(n("#,##0", 999.0), "999");
        assert_eq!(n("#,##0", 0.0), "0");
        assert_eq!(n("#,##0.00", 1234.5), "1,234.50");
        assert_eq!(n("#,##0.00", -1234.567), "-1,234.57");
        assert_eq!(n("0,000", 5.0), "0,005");
        assert_eq!(n("#,##0,", 1234567.0), "1,235");
        assert_eq!(n("0.0,,\"M\"", 12_345_678.0), "12.3M");
        assert_eq!(n("0,", 1500.0), "2");
        assert_eq!(n("#,###", 1000.0), "1,000");
        assert_eq!(n("#,###", 12.0), "12");
    }

    #[test]
    fn percent_and_exp() {
        assert_eq!(n("0%", 0.256), "26%");
        assert_eq!(n("0.00%", 0.12345), "12.35%");
        assert_eq!(n("0.0%", 0.07), "7.0%");
        assert_eq!(n("0.00E+00", 12345.0), "1.23E+04");
        assert_eq!(n("0.00E+00", 0.000123), "1.23E-04");
        assert_eq!(n("0.0E+0", 5.0), "5.0E+0");
        assert_eq!(n("0.00E-00", 12345.0), "1.23E04");
        assert_eq!(n("##0.0E+0", 12345.0), "12.3E+3");
        assert_eq!(n("0.00E+00", 0.0), "0.00E+00");
        assert_eq!(n("0.0E+00", 9.99e5), "1.0E+06");
        assert_eq!(n("0.00E+00", -12345.0), "-1.23E+04");
    }

    #[test]
    fn literals_and_text() {
        assert_eq!(n("\"¥\"#,##0.00", 1234.5), "¥1,234.50");
        assert_eq!(n("\"¥\"#,##0.00", -1234.5), "-¥1,234.50");
        assert_eq!(n("0\"元\"", 5.0), "5元");
        assert_eq!(n("\\$0.00", 3.0), "$3.00");
        assert_eq!(n("[$¥-804]#,##0.00", 99.5), "¥99.50");
        assert_eq!(n("[$$-409]#,##0", 1200.0), "$1,200");
        assert_eq!(n("[$-409]0.0", 1.25), "1.3");
        assert_eq!(n("000-0000-0000", 13812345678.0), "138-1234-5678");
        assert_eq!(n("(###) ###-####", 5551234567.0), "(555) 123-4567");
        assert_eq!(n("0_);(0)", 5.0), "5 ");
        assert_eq!(n("0_);(0)", -5.0), "(5)");
        assert_eq!(n("\"abc\"", 5.0), "abc");
        assert_eq!(n("0 \"kg\"", 5.0), "5 kg");
        assert_eq!(t("@", "hi"), "hi");
        assert_eq!(t("\"前缀\"@", "x"), "前缀x");
        assert_eq!(t("0;0;0;\"[\"@\"]\"", "x"), "[x]");
        assert_eq!(t("0.00", "abc"), "abc");
        assert_eq!(n("@", 12.5), "12.5");
    }

    #[test]
    fn sections_and_conditions() {
        assert_eq!(n("0;(0)", 5.0), "5");
        assert_eq!(n("0;(0)", -5.0), "(5)");
        assert_eq!(n("0;(0)", 0.0), "0");
        assert_eq!(n("0;-0;\"零\"", 0.0), "零");
        assert_eq!(n("0;-0;\"零\"", 7.0), "7");
        assert_eq!(n("0;-0;\"零\"", -7.0), "-7");
        assert_eq!(n("[>100]\"big\";\"small\"", 150.0), "big");
        assert_eq!(n("[>100]\"big\";\"small\"", 50.0), "small");
        assert_eq!(n("[<1000]0;[<1000000]0.0,\"K\";0.0,,\"M\"", 500.0), "500");
        assert_eq!(n("[<1000]0;[<1000000]0.0,\"K\";0.0,,\"M\"", 25_000.0), "25.0K");
        assert_eq!(n("[<1000]0;[<1000000]0.0,\"K\";0.0,,\"M\"", 2_500_000.0), "2.5M");
        assert_eq!(n("[=1]\"one\";[=2]\"two\";\"many\"", 2.0), "two");
        assert_eq!(n("#,##0;[Red]-#,##0", -1234.0), "-1,234");
        assert_eq!(n(";;;", 5.0), "");
        assert_eq!(n("0;;", -5.0), "");
    }

    #[test]
    fn colors() {
        let mut s = String::new();
        let c = parse("[Red]0.0;[Blue]-0.0").number(-2.0, false, &mut s);
        assert_eq!(s, "-2.0");
        assert_eq!(c, Some(Rgb(0, 0, 255)));
        s.clear();
        assert_eq!(parse("[Red]0").number(1.0, false, &mut s), Some(Rgb(255, 0, 0)));
        s.clear();
        assert_eq!(parse("[Green]0").number(1.0, false, &mut s), Some(Rgb(0, 255, 0)));
        s.clear();
        assert_eq!(parse("0").number(1.0, false, &mut s), None);
        s.clear();
        assert_eq!(parse("[Color 3]0").number(1.0, false, &mut s), Some(Rgb(255, 0, 0)));
    }

    #[test]
    fn builtins() {
        let f = |id: u32, v: f64| {
            let mut s = String::new();
            parse(builtin(id).unwrap()).number(v, false, &mut s);
            s
        };
        assert_eq!(f(1, 12.6), "13");
        assert_eq!(f(2, 12.6), "12.60");
        assert_eq!(f(3, 1234567.0), "1,234,567");
        assert_eq!(f(4, 1234.5), "1,234.50");
        assert_eq!(f(9, 0.5), "50%");
        assert_eq!(f(10, 0.1234), "12.34%");
        assert_eq!(f(11, 12345.0), "1.23E+04");
        assert_eq!(f(37, -1234.0), "(1,234)");
        assert_eq!(f(37, 1234.0), "1,234 ");
        assert_eq!(f(5, 12.0), "¥12 ");
        assert_eq!(f(8, -12.5), "(¥12.50)");
        assert_eq!(f(41, 1234.0), " 1,234 ");
        assert_eq!(f(41, 0.0), " - ");
        assert_eq!(f(48, 12345.0), "12.3E+3");
        assert_eq!(f(49, 5.0), "5");
        assert_eq!(f(14, ser(2024, 3, 5)), "2024/3/5");
        assert_eq!(f(22, ser(2024, 3, 5) + 0.5625), "2024/3/5 13:30");
        assert_eq!(f(15, ser(2024, 3, 5)), "5-Mar-24");
        assert_eq!(f(16, ser(2024, 3, 5)), "5-Mar");
        assert_eq!(f(17, ser(2024, 3, 5)), "Mar-24");
        assert_eq!(f(18, 0.5625), "1:30 PM");
        assert_eq!(f(19, 0.0), "12:00:00 AM");
        assert_eq!(f(20, 0.5625), "13:30");
        assert_eq!(f(21, 0.5625), "13:30:00");
        assert_eq!(f(27, ser(2024, 3, 5)), "2024年3月");
        assert_eq!(f(31, ser(2024, 3, 5)), "2024年3月5日");
        assert_eq!(f(28, ser(2024, 3, 5)), "3月5日");
        assert_eq!(f(32, 0.5625), "13时30分");
        assert_eq!(f(33, 0.5625), "13时30分00秒");
        assert_eq!(f(34, 0.5625), "下午1时30分");
        assert_eq!(f(35, 0.25), "上午6时00分00秒");
        assert_eq!(f(45, 0.0105), "15:07");
        assert_eq!(f(46, 1.5), "36:00:00");
        assert_eq!(f(47, 0.0001), "00:08.6");
        assert!(builtin(23).is_none());
    }

    #[test]
    fn dates() {
        assert_eq!(n("yyyy-mm-dd", 45000.0), "2023-03-15");
        assert_eq!(n("yyyy/m/d", 1.0), "1900/1/1");
        assert_eq!(n("yyyy/m/d", 59.0), "1900/2/28");
        assert_eq!(n("yyyy/m/d", 60.0), "1900/2/29");
        assert_eq!(n("yyyy/m/d", 61.0), "1900/3/1");
        assert_eq!(n("yyyy/m/d", 0.0), "1900/1/0");
        assert_eq!(n("yyyy-mm-dd", 36526.0), "2000-01-01");
        assert_eq!(n("yy.m.d", 45000.0), "23.3.15");
        assert_eq!(n("mmmm d, yyyy", 45000.0), "March 15, 2023");
        assert_eq!(n("mmm", 45000.0), "Mar");
        assert_eq!(n("mmmmm", 45000.0), "M");
        assert_eq!(n("dddd", 45000.0), "Wednesday");
        assert_eq!(n("ddd", 45000.0), "Wed");
        assert_eq!(n("dddd", 1.0), "Sunday");
        assert_eq!(n("dddd", 61.0), "Thursday");
        assert_eq!(n("aaaa", 45000.0), "星期三");
        assert_eq!(n("aaa", 45000.0), "三");
        assert_eq!(n("[$-804]dddd", 45000.0), "星期三");
        assert_eq!(n("[$-804]mmmm", 45000.0), "三月");
        assert_eq!(n("yyyy\"年\"m\"月\"d\"日\"", 45000.0), "2023年3月15日");
        assert_eq!(n("[$-F800]dddd, mmmm dd, yyyy", 45000.0), "Wednesday, March 15, 2023");
        assert_eq!(n("yyyy/m/d", -1.0), "########");
        assert_eq!(n("yyyy/m/d", 3_000_000.0), "########");
    }

    #[test]
    fn times() {
        assert_eq!(n("h:mm", 0.5), "12:00");
        assert_eq!(n("hh:mm:ss", 0.75), "18:00:00");
        assert_eq!(n("h:mm AM/PM", 0.75), "6:00 PM");
        assert_eq!(n("h:mm am/pm", 0.25), "6:00 am");
        assert_eq!(n("h:mm A/P", 0.75), "6:00 P");
        assert_eq!(n("h:mm:ss AM/PM", 0.0), "12:00:00 AM");
        assert_eq!(n("hh:mm", 0.999999), "00:00");
        assert_eq!(n("yyyy-mm-dd hh:mm:ss", 45000.75), "2023-03-15 18:00:00");
        assert_eq!(n("yyyy/m/d h:mm", 45000.0 + 59.0 / 1440.0), "2023/3/15 0:59");
        assert_eq!(n("mm:ss", 0.0105), "15:07");
        assert_eq!(n("mm:ss.0", 0.0001), "00:08.6");
        assert_eq!(n("h:mm:ss.00", 0.5 + 0.123 / 86400.0), "12:00:00.12");
        assert_eq!(n("[h]:mm:ss", 1.5), "36:00:00");
        assert_eq!(n("[h]:mm", 2.0 / 24.0 + 5.0 / 1440.0), "2:05");
        assert_eq!(n("[mm]:ss", 0.01), "14:24");
        assert_eq!(n("[ss]", 0.001), "86");
        assert_eq!(n("上午/下午h:mm", 0.75), "下午6:00");
        // m after h is minutes, m alone is a month
        assert_eq!(n("m", 45000.0), "3");
        assert_eq!(n("h m", 0.5 + 7.0 / 1440.0), "12 7");
        assert_eq!(n("m/d/yy", 45000.0), "3/15/23");
    }

    #[test]
    fn date1904() {
        let mut s = String::new();
        parse("yyyy-mm-dd").number(0.0, true, &mut s);
        assert_eq!(s, "1904-01-01");
        s.clear();
        parse("yyyy-mm-dd").number(1462.0, true, &mut s);
        assert_eq!(s, "1908-01-02");
        assert_eq!(serial_from_ymd(1904, 1, 1, true), 0.0);
        assert_eq!(serial_from_ymd(2023, 3, 15, false), 45000.0);
        s.clear();
        parse("dddd").number(0.0, true, &mut s);
        assert_eq!(s, "Friday");
    }

    #[test]
    fn fractions() {
        assert_eq!(n("# ?/?", 1.5), "1 1/2");
        assert_eq!(n("# ?/?", 0.5), " 1/2");
        assert_eq!(n("# ?/?", 3.0), "3    ");
        assert_eq!(n("# ??/??", 0.3333333), "  1/3 ");
        assert_eq!(n("?/?", 0.75), "3/4");
        assert_eq!(n("?/?", 1.75), "7/4");
        assert_eq!(n("# ?/8", 1.25), "1 2/8");
        assert_eq!(n("# ??/??", 1.25), "1  1/4 ");
        assert_eq!(n("# ?/?", 2.9999), "3    ");
        assert_eq!(n("?/?", 0.0), "0/1");
    }

    #[test]
    fn wps_style_formats() {
        let acct = "_ * #,##0_ ;_ * \\-#,##0_ ;_ * \"-\"_ ;_ @_ ";
        assert_eq!(n(acct, 1234.0), " 1,234 ");
        assert_eq!(n(acct, -1234.0), " -1,234 ");
        assert_eq!(n(acct, 0.0), " - ");
        assert_eq!(t(acct, "abc"), " abc ");
        let cny = "_ \"￥\"* #,##0.00_ ;_ \"￥\"* \\-#,##0.00_ ;_ \"￥\"* \"-\"??_ ;_ @_ ";
        assert_eq!(n(cny, 12.5), " ￥12.50 ");
        assert_eq!(n(cny, -3.0), " ￥-3.00 ");
        assert_eq!(n(cny, 0.0), " ￥-   ");
        assert_eq!(n("yyyy-MM-dd HH:mm:ss", 45000.75), "2023-03-15 18:00:00");
        assert_eq!(n("HH:MM:SS", 0.75), "18:00:00");
        assert_eq!(n("0.0_);[Red](0.0)", -1.25), "(1.3)");
        assert_eq!(n("#,##0.00_);[Red](#,##0.00)", 1234.567), "1,234.57 ");
    }

    #[test]
    fn misc() {
        assert_eq!(n("General\" units\"", 5.0), "5 units");
        assert!(parse("yyyy/m/d").is_date());
        assert!(!parse("0.00").is_date());
        assert!(parse("General").is_general());
        assert!(parse("@").has_text_section());
        assert!(!parse("0.00").has_text_section());
        let mut s = String::new();
        general(f64::NAN, &mut s);
        assert_eq!(s, "#NUM!");
    }
}
