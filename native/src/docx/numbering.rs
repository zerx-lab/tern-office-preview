//! `numbering.xml`: list definitions, level counters and the label text of each list paragraph.

use super::props::{PPr, RPr};
use crate::color::Theme;
use crate::xml::{self, attr, child, children, elems, num, on_off, wval};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Suff {
    Tab,
    Space,
    Nothing,
}

#[derive(Clone, Debug)]
struct Lvl {
    start: i64,
    fmt: String,
    text: String,
    suff: Suff,
    ppr: PPr,
    rpr: RPr,
    is_lgl: bool,
    pstyle: Option<String>,
    /// Font of the level's `rPr` (decides how a bullet glyph is read).
    symbol_font: Option<String>,
}

#[derive(Default)]
struct Abstract {
    lvls: [Option<Lvl>; 9],
}

#[derive(Default)]
struct NumDef {
    abs: i64,
    start_override: HashMap<usize, i64>,
    lvl_override: HashMap<usize, Lvl>,
}

/// What a list paragraph shows in front of its text.
#[derive(Clone, Debug)]
pub struct Label {
    pub text: String,
    pub suff: Suff,
    pub ppr: PPr,
    pub rpr: RPr,
}

#[derive(Default)]
pub struct Numbering {
    abstracts: HashMap<i64, Abstract>,
    nums: HashMap<i64, NumDef>,
    counters: HashMap<i64, [Option<i64>; 9]>,
    started: HashSet<i64>,
}

fn parse_lvl(n: roxmltree::Node, theme: &Theme) -> Lvl {
    let mut l = Lvl {
        start: 1,
        fmt: "decimal".into(),
        text: String::new(),
        suff: Suff::Tab,
        ppr: PPr::default(),
        rpr: RPr::default(),
        is_lgl: false,
        pstyle: None,
        symbol_font: None,
    };
    for c in elems(n) {
        let v = wval(c);
        match xml::qname(c).as_str() {
            "w:start" => l.start = num(v).unwrap_or(1),
            "w:numFmt" => l.fmt = v.unwrap_or("decimal").to_string(),
            "w:lvlText" => l.text = v.unwrap_or("").to_string(),
            "w:suff" => {
                l.suff = match v {
                    Some("space") => Suff::Space,
                    Some("nothing") => Suff::Nothing,
                    _ => Suff::Tab,
                }
            }
            "w:isLgl" => l.is_lgl = on_off(v),
            "w:pStyle" => l.pstyle = v.map(str::to_string),
            "w:pPr" => l.ppr = PPr::parse(c, theme),
            "w:rPr" => {
                l.rpr = RPr::parse(c, theme);
                l.symbol_font = child(c, "w", "rFonts").and_then(|f| attr(f, "w", "ascii").or_else(|| attr(f, "w", "hAnsi")))
                    .map(str::to_string);
            }
            _ => {}
        }
    }
    l
}

impl Numbering {
    pub fn parse(xml_text: Option<&str>, theme: &Theme) -> Numbering {
        let mut nb = Numbering::default();
        let Some(text) = xml_text else { return nb };
        let Ok(doc) = roxmltree::Document::parse_with_options(text, xml::OPTS) else { return nb };
        let root = doc.root_element();
        for a in children(root, "w", "abstractNum") {
            let Some(id) = num::<i64>(attr(a, "w", "abstractNumId")) else { continue };
            let mut abs = Abstract::default();
            for (pos, l) in children(a, "w", "lvl").enumerate() {
                let i = num::<usize>(attr(l, "w", "ilvl")).unwrap_or(pos);
                if i < 9 {
                    abs.lvls[i] = Some(parse_lvl(l, theme));
                }
            }
            nb.abstracts.insert(id, abs);
        }
        for n in children(root, "w", "num") {
            let Some(id) = num::<i64>(attr(n, "w", "numId")) else { continue };
            let Some(abs) = child(n, "w", "abstractNumId").and_then(wval).and_then(|v| num::<i64>(Some(v))) else {
                continue;
            };
            let mut def = NumDef { abs, ..Default::default() };
            for o in children(n, "w", "lvlOverride") {
                let Some(i) = num::<usize>(attr(o, "w", "ilvl")).filter(|i| *i < 9) else { continue };
                if let Some(s) = child(o, "w", "startOverride").and_then(wval).and_then(|v| num::<i64>(Some(v))) {
                    def.start_override.insert(i, s);
                }
                if let Some(l) = child(o, "w", "lvl") {
                    def.lvl_override.insert(i, parse_lvl(l, theme));
                }
            }
            nb.nums.insert(id, def);
        }
        nb
    }

    fn lvl(&self, num_id: i64, i: usize) -> Option<&Lvl> {
        let def = self.nums.get(&num_id)?;
        def.lvl_override.get(&i).or_else(|| self.abstracts.get(&def.abs)?.lvls[i].as_ref())
    }

    /// The level of list `num_id` that style `style` is linked to (`w:lvl/w:pStyle`).
    pub fn level_for_style(&self, num_id: u32, style: &str) -> Option<u8> {
        let def = self.nums.get(&(num_id as i64))?;
        let abs = self.abstracts.get(&def.abs)?;
        abs.lvls.iter().position(|l| l.as_ref().is_some_and(|l| l.pstyle.as_deref() == Some(style))).map(|i| i as u8)
    }

    /// Advances list `num_id` at level `ilvl` and returns the label to show, or `None` when the
    /// list or level is undefined (or `numId` 0).
    pub fn label(&mut self, num_id: u32, ilvl: u8) -> Option<Label> {
        let num_id = num_id as i64;
        let i = (ilvl as usize).min(8);
        let def = self.nums.get(&num_id)?;
        let abs_id = def.abs;
        let lvl = self.lvl(num_id, i)?.clone();
        // A list instance restarts the levels its `startOverride`s name when first used.
        if self.started.insert(num_id) {
            let starts: Vec<(usize, i64)> = self.nums[&num_id].start_override.iter().map(|(k, v)| (*k, *v)).collect();
            let c = self.counters.entry(abs_id).or_insert([None; 9]);
            for (l, s) in starts {
                c[l] = Some(s - 1);
                for d in c.iter_mut().skip(l + 1) {
                    *d = None;
                }
            }
        }
        let bullet = lvl.fmt == "bullet";
        let c = self.counters.entry(abs_id).or_insert([None; 9]);
        if !bullet {
            c[i] = Some(match c[i] {
                Some(v) => v + 1,
                None => lvl.start,
            });
            for d in c.iter_mut().skip(i + 1) {
                *d = None;
            }
        }
        let counters = *c;
        let text = if bullet {
            bullet_char(&lvl.text, lvl.symbol_font.as_deref())
        } else {
            let mut out = String::new();
            let mut chars = lvl.text.chars().peekable();
            while let Some(ch) = chars.next() {
                if ch == '%' {
                    if let Some(d) = chars.peek().and_then(|d| d.to_digit(10)).filter(|d| (1..=9).contains(d)) {
                        chars.next();
                        let k = d as usize - 1;
                        let lv = self.lvl(num_id, k);
                        let val = counters[k].unwrap_or_else(|| lv.map_or(1, |l| l.start));
                        let fmt = match lv {
                            Some(l) if !(lvl.is_lgl && l.fmt != "bullet") => l.fmt.as_str(),
                            _ => "decimal",
                        };
                        out.push_str(&format_number(val, fmt));
                        continue;
                    }
                }
                out.push(ch);
            }
            out
        };
        Some(Label { text, suff: lvl.suff, ppr: lvl.ppr, rpr: lvl.rpr })
    }
}

/// A bullet's glyph: Symbol / Wingdings private-use characters become Unicode look-alikes.
pub fn bullet_char(text: &str, font: Option<&str>) -> String {
    let mut out = String::new();
    for c in text.chars() {
        out.push_str(match c {
            '\u{F0B7}' | '\u{00B7}' | '\u{2022}' => "•",
            '\u{F0A7}' | '\u{25AA}' => "▪",
            '\u{F0D8}' => "➢",
            '\u{F076}' => "❖",
            '\u{F0FC}' => "✓",
            '\u{F0FB}' => "✗",
            '\u{F06E}' => "■",
            '\u{F06F}' => "□",
            '\u{F071}' => "❑",
            '\u{F0A8}' => "◻",
            '\u{F0E8}' => "➔",
            '\u{F0D8}'..='\u{F0DF}' => "➢",
            'o' if font.is_some_and(|f| f.eq_ignore_ascii_case("Courier New")) => "○",
            'n' if font.is_some_and(|f| f.to_ascii_lowercase().contains("wingdings")) => "■",
            'l' if font.is_some_and(|f| f.to_ascii_lowercase().contains("wingdings")) => "●",
            'q' if font.is_some_and(|f| f.to_ascii_lowercase().contains("wingdings")) => "❑",
            'v' if font.is_some_and(|f| f.to_ascii_lowercase().contains("wingdings")) => "❖",
            '§' if font.is_some_and(|f| f.to_ascii_lowercase().contains("wingdings")) => "▪",
            'Ø' if font.is_some_and(|f| f.to_ascii_lowercase().contains("wingdings")) => "➢",
            'ü' if font.is_some_and(|f| f.to_ascii_lowercase().contains("wingdings")) => "✓",
            c if ('\u{F000}'..='\u{F0FF}').contains(&c) => "•",
            c => {
                out.push(c);
                continue;
            }
        });
    }
    out
}

pub fn format_number(n: i64, fmt: &str) -> String {
    let u = n.max(0) as u64;
    match fmt {
        "decimal" | "decimalHalfWidth" => n.to_string(),
        "decimalZero" => format!("{n:02}"),
        "decimalFullWidth" | "decimalFullWidth2" => n.to_string().chars().map(fullwidth_digit).collect(),
        "upperRoman" => roman(u).unwrap_or_else(|| n.to_string()),
        "lowerRoman" => roman(u).map(|s| s.to_lowercase()).unwrap_or_else(|| n.to_string()),
        "upperLetter" => letters(u, b'A'),
        "lowerLetter" => letters(u, b'a'),
        "chineseCounting" | "chineseCountingThousand" | "japaneseCounting" | "koreanCounting" | "taiwaneseCounting"
        | "taiwaneseCountingThousand" | "chineseLegal" => chinese(u, &CN_DIGITS, &CN_UNITS, true),
        "chineseLegalSimplified" | "ideographLegalTraditional" => chinese(u, &CN_LEGAL_DIGITS, &CN_LEGAL_UNITS, false),
        "ideographDigital" | "japaneseDigitalTenThousand" | "koreanDigital" | "taiwaneseDigital" => {
            n.to_string().chars().map(|c| c.to_digit(10).map_or(c, |d| CN_DIGITS_0[d as usize])).collect()
        }
        "ideographTraditional" => nth(&"甲乙丙丁戊己庚辛壬癸".chars().collect::<Vec<_>>(), u, n),
        "ideographZodiac" => nth(&"子丑寅卯辰巳午未申酉戌亥".chars().collect::<Vec<_>>(), u, n),
        "decimalEnclosedCircle" | "decimalEnclosedCircleChinese" => match u {
            1..=20 => char::from_u32(0x2460 + (u as u32 - 1)).map(String::from).unwrap_or_default(),
            21..=35 => char::from_u32(0x3251 + (u as u32 - 21)).map(String::from).unwrap_or_default(),
            _ => n.to_string(),
        },
        "decimalEnclosedParen" => match u {
            1..=20 => char::from_u32(0x2474 + (u as u32 - 1)).map(String::from).unwrap_or_default(),
            _ => format!("({n})"),
        },
        "decimalEnclosedFullstop" => match u {
            1..=20 => char::from_u32(0x2488 + (u as u32 - 1)).map(String::from).unwrap_or_default(),
            _ => format!("{n}."),
        },
        "ordinal" => {
            let suf = match (u % 100, u % 10) {
                (11..=13, _) => "th",
                (_, 1) => "st",
                (_, 2) => "nd",
                (_, 3) => "rd",
                _ => "th",
            };
            format!("{n}{suf}")
        }
        "none" => String::new(),
        _ => n.to_string(),
    }
}

fn nth(set: &[char], u: u64, n: i64) -> String {
    if u >= 1 && (u as usize) <= set.len() {
        set[u as usize - 1].to_string()
    } else {
        n.to_string()
    }
}

fn fullwidth_digit(c: char) -> char {
    match c.to_digit(10) {
        Some(d) => char::from_u32('０' as u32 + d).unwrap_or(c),
        None => c,
    }
}

fn roman(mut n: u64) -> Option<String> {
    if n == 0 || n >= 4000 {
        return None;
    }
    const T: [(u64, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut s = String::new();
    for (v, r) in T {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    Some(s)
}

/// Word's letter lists: A…Z, then AA, BB, CC…
fn letters(n: u64, base: u8) -> String {
    if n == 0 {
        return "0".into();
    }
    let ch = (base + ((n - 1) % 26) as u8) as char;
    let rep = (n - 1) / 26 + 1;
    std::iter::repeat_n(ch, rep as usize).collect()
}

const CN_DIGITS: [char; 10] = ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
const CN_DIGITS_0: [char; 10] = ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
const CN_UNITS: [&str; 4] = ["", "十", "百", "千"];
const CN_LEGAL_DIGITS: [char; 10] = ['零', '壹', '贰', '叁', '肆', '伍', '陆', '柒', '捌', '玖'];
const CN_LEGAL_UNITS: [&str; 4] = ["", "拾", "佰", "仟"];

/// Chinese numerals up to 99999999 (`十一`, `二十`, `一百零五`); `short_ten` writes 10–19 as `十X`.
fn chinese(n: u64, digits: &[char; 10], units: &[&str; 4], short_ten: bool) -> String {
    if n == 0 {
        return digits[0].to_string();
    }
    if n >= 100_000_000 {
        return n.to_string();
    }
    let section = |mut v: u64, lead_short: bool| -> String {
        let mut s = String::new();
        let mut zero = false;
        let mut started = false;
        for p in (0..4).rev() {
            let div = 10u64.pow(p);
            let d = v / div;
            v %= div;
            if d == 0 {
                if started {
                    zero = true;
                }
                continue;
            }
            if zero {
                s.push(digits[0]);
                zero = false;
            }
            if !(d == 1 && p == 1 && !started && lead_short) {
                s.push(digits[d as usize]);
            }
            s.push_str(units[p as usize]);
            started = true;
        }
        s
    };
    let hi = n / 10_000;
    let lo = n % 10_000;
    if hi == 0 {
        return section(lo, short_ten);
    }
    let mut s = section(hi, short_ten);
    s.push('万');
    if lo > 0 {
        if lo < 1000 {
            s.push(digits[0]);
        }
        s.push_str(&section(lo, false));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

    fn nb(xml: &str) -> Numbering {
        Numbering::parse(Some(&format!("<w:numbering {NS}>{xml}</w:numbering>")), &Theme::default())
    }

    fn labels(n: &mut Numbering, seq: &[(u32, u8)]) -> Vec<String> {
        seq.iter().map(|(id, l)| n.label(*id, *l).unwrap().text).collect()
    }

    #[test]
    fn decimal_multilevel_resets_deeper_levels() {
        let mut n = nb(r#"<w:abstractNum w:abstractNumId="0">
            <w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl>
            <w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1.%2"/></w:lvl>
            <w:lvl w:ilvl="2"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="(%3)"/></w:lvl>
            </w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#);
        let got = labels(&mut n, &[(1, 0), (1, 1), (1, 1), (1, 2), (1, 2), (1, 0), (1, 1), (1, 2)]);
        assert_eq!(got, ["1.", "1.1", "1.2", "(a)", "(b)", "2.", "2.1", "(a)"]);
    }

    #[test]
    fn roman_chinese_and_start_override() {
        let mut n = nb(r#"<w:abstractNum w:abstractNumId="0">
            <w:lvl w:ilvl="0"><w:start w:val="4"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="%1)"/></w:lvl>
            </w:abstractNum>
            <w:abstractNum w:abstractNumId="1">
            <w:lvl w:ilvl="0"><w:numFmt w:val="chineseCounting"/><w:lvlText w:val="第%1章"/></w:lvl>
            <w:lvl w:ilvl="1"><w:numFmt w:val="chineseCountingThousand"/><w:lvlText w:val="（%2）"/></w:lvl>
            </w:abstractNum>
            <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
            <w:num w:numId="2"><w:abstractNumId w:val="0"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="1"/></w:lvlOverride></w:num>
            <w:num w:numId="3"><w:abstractNumId w:val="1"/></w:num>"#);
        assert_eq!(labels(&mut n, &[(1, 0), (1, 0)]), ["IV)", "V)"]);
        // A second instance with a startOverride restarts the shared counter.
        assert_eq!(labels(&mut n, &[(2, 0), (2, 0)]), ["I)", "II)"]);
        let got = labels(&mut n, &[(3, 0), (3, 1), (3, 1), (3, 1)]);
        assert_eq!(got, ["第一章", "（一）", "（二）", "（三）"]);
    }

    #[test]
    fn chinese_numbers() {
        let c = |n| format_number(n, "chineseCounting");
        assert_eq!(c(10), "十");
        assert_eq!(c(11), "十一");
        assert_eq!(c(20), "二十");
        assert_eq!(c(21), "二十一");
        assert_eq!(c(100), "一百");
        assert_eq!(c(105), "一百零五");
        assert_eq!(c(110), "一百一十");
        assert_eq!(c(1001), "一千零一");
        assert_eq!(format_number(3, "ideographTraditional"), "丙");
        assert_eq!(format_number(3, "decimalEnclosedCircle"), "③");
        assert_eq!(format_number(7, "decimalZero"), "07");
        assert_eq!(format_number(28, "upperLetter"), "BB");
        assert_eq!(format_number(14, "lowerRoman"), "xiv");
    }

    #[test]
    fn bullets_map_symbol_glyphs() {
        let mut n = nb(r#"<w:abstractNum w:abstractNumId="0">
            <w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val="&#xF0B7;"/><w:rPr><w:rFonts w:ascii="Symbol"/></w:rPr></w:lvl>
            <w:lvl w:ilvl="1"><w:numFmt w:val="bullet"/><w:lvlText w:val="o"/><w:rPr><w:rFonts w:ascii="Courier New"/></w:rPr></w:lvl>
            <w:lvl w:ilvl="2"><w:numFmt w:val="bullet"/><w:lvlText w:val="&#xF0A7;"/></w:lvl>
            <w:lvl w:ilvl="3"><w:numFmt w:val="bullet"/><w:lvlText w:val="&#xF0D8;"/></w:lvl>
            <w:lvl w:ilvl="4"><w:numFmt w:val="bullet"/><w:lvlText w:val="&#xF0FC;"/></w:lvl>
            </w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#);
        assert_eq!(labels(&mut n, &[(1, 0), (1, 1), (1, 2), (1, 3), (1, 4)]), ["•", "○", "▪", "➢", "✓"]);
    }

    #[test]
    fn style_linked_level() {
        let n = nb(r#"<w:abstractNum w:abstractNumId="0">
            <w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/><w:lvlText w:val="%1"/><w:pStyle w:val="H1"/></w:lvl>
            <w:lvl w:ilvl="1"><w:numFmt w:val="decimal"/><w:lvlText w:val="%1.%2"/><w:pStyle w:val="H2"/></w:lvl>
            </w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#);
        assert_eq!(n.level_for_style(1, "H2"), Some(1));
        assert_eq!(n.level_for_style(1, "Body"), None);
    }
}
