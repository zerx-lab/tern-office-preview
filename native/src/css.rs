//! The plugin's style sheet and the utility classes renderers put on elements.
//!
//! Tern views can't carry inline styles, so every visual property a document needs is a class
//! from a fixed, generated set: colors quantized to an 8-level cube, lengths quantized to a few
//! px. `stylesheet()` is what `office.css` holds (a test keeps the two in step); the helpers
//! below are the only way renderers name those classes.
//!
//! Every rule sits under `.opv`, the class of a rendered part's root. Base rules (tags, layout)
//! come first at specificity ≤ (0,2,0); utilities come after at (0,2,0), so a utility always wins.

use crate::color::Rgb;
use std::fmt::Write;

// ---- quantization -------------------------------------------------------------------------

const C8: [u8; 8] = [0, 36, 73, 109, 146, 182, 219, 255];
const C4: [u8; 4] = [0, 85, 170, 255];

fn q(v: u8, levels: usize) -> usize {
    ((v as f64) * (levels as f64 - 1.0) / 255.0).round() as usize
}

/// Text color class (`c707`).
pub fn fg(c: Rgb) -> String {
    format!("c{}{}{}", q(c.0, 8), q(c.1, 8), q(c.2, 8))
}

/// Background color class (`k707`).
pub fn bg(c: Rgb) -> String {
    format!("k{}{}{}", q(c.0, 8), q(c.1, 8), q(c.2, 8))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    fn letter(self) -> char {
        match self {
            Side::Top => 't',
            Side::Right => 'r',
            Side::Bottom => 'b',
            Side::Left => 'l',
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Line {
    /// 1px solid.
    Thin,
    /// 2px solid.
    Medium,
    /// 3px solid.
    Thick,
    Dashed,
    Dotted,
    Double,
}

impl Line {
    fn code(self) -> char {
        match self {
            Line::Thin => '1',
            Line::Medium => '2',
            Line::Thick => '3',
            Line::Dashed => 'd',
            Line::Dotted => 'o',
            Line::Double => 'w',
        }
    }

    fn css(self) -> &'static str {
        match self {
            Line::Thin => "1px solid",
            Line::Medium => "2px solid",
            Line::Thick => "3px solid",
            Line::Dashed => "1px dashed",
            Line::Dotted => "1px dotted",
            Line::Double => "3px double",
        }
    }

    const ALL: [Line; 6] = [Line::Thin, Line::Medium, Line::Thick, Line::Dashed, Line::Dotted, Line::Double];
}

/// One side's border: a line and its color (black when `None`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Border {
    pub line: Line,
    pub color: Option<Rgb>,
}

/// Classes for one side's border (`bt1 bct012`), appended to `out` with a leading space.
pub fn border(out: &mut String, side: Side, b: Border) {
    let s = side.letter();
    let _ = write!(out, " b{s}{}", b.line.code());
    if let Some(c) = b.color {
        if c != Rgb::BLACK {
            let _ = write!(out, " bc{s}{}{}{}", q(c.0, 4), q(c.1, 4), q(c.2, 4));
        }
    }
}

/// Class that removes a side's border (overrides gridlines and base rules).
pub fn no_border(side: Side) -> String {
    format!("b{}n", side.letter())
}

fn clamp_step(v: f64, step: f64, max: u32) -> Option<u32> {
    if !v.is_finite() || v <= 0.0 {
        return None;
    }
    Some(((v / step).round() as u32).clamp(1, max))
}

/// Font size in half-points (Word `w:sz`, Excel `sz` × 2): `f22`.
pub fn size(half_points: f64) -> Option<String> {
    clamp_step(half_points, 1.0, 200).map(|n| format!("f{}", n.max(2)))
}

/// Fixed width in px (4px steps): `w16`.
pub fn width(px: f64) -> Option<String> {
    clamp_step(px, 4.0, 600).map(|n| format!("w{n}"))
}

/// Maximum width in px (4px steps): `mw150`.
pub fn max_width(px: f64) -> Option<String> {
    clamp_step(px, 4.0, 600).map(|n| format!("mw{n}"))
}

/// Margin above / below in px (2px steps): `mt6`, `mb4`.
pub fn margin_top(px: f64) -> Option<String> {
    clamp_step(px, 2.0, 100).map(|n| format!("mt{n}"))
}
pub fn margin_bottom(px: f64) -> Option<String> {
    clamp_step(px, 2.0, 100).map(|n| format!("mb{n}"))
}

/// Padding left / right in px (4px steps): `pl9`, `pr3`.
pub fn pad_left(px: f64) -> Option<String> {
    clamp_step(px, 4.0, 200).map(|n| format!("pl{n}"))
}
pub fn pad_right(px: f64) -> Option<String> {
    clamp_step(px, 4.0, 200).map(|n| format!("pr{n}"))
}
/// Padding top / bottom in px (4px steps): `pt12`.
pub fn pad_top(px: f64) -> Option<String> {
    clamp_step(px, 4.0, 50).map(|n| format!("pt{n}"))
}
pub fn pad_bottom(px: f64) -> Option<String> {
    clamp_step(px, 4.0, 50).map(|n| format!("pb{n}"))
}

/// Negative left margin in px (4px steps), for hanging indents: `ml6`.
pub fn pull_left(px: f64) -> Option<String> {
    clamp_step(px, 4.0, 100).map(|n| format!("ml{n}"))
}

/// Line height as a multiple of the font size (5% steps): `lh23` is 1.15.
pub fn line_mult(mult: f64) -> Option<String> {
    clamp_step(mult, 0.05, 80).map(|n| format!("lh{}", n.max(10)))
}

/// Line height in px: `lp20`.
pub fn line_px(px: f64) -> Option<String> {
    clamp_step(px, 1.0, 160).map(|n| format!("lp{}", n.max(6)))
}

/// Row height in px (2px steps): `h10`.
pub fn height(px: f64) -> Option<String> {
    clamp_step(px, 2.0, 300).map(|n| format!("h{n}"))
}

/// Excel cell indent level (`alignment indent`): `xi2`.
pub fn indent_level(n: u32) -> Option<String> {
    if n == 0 {
        None
    } else {
        Some(format!("xi{}", n.min(15)))
    }
}

/// The grid container's column count (`gt9`): `repeat(n, max-content)` tracks, so each column
/// is as wide as its cells' fixed `width` classes. Tern's table layout ignores colspan/rowspan,
/// which is why sheets and Word tables are grids.
pub fn grid_cols(n: usize) -> String {
    format!("gt{}", n.clamp(1, 260))
}

/// A cell spanning `n` columns (`gs3`); `None` for 1.
pub fn col_span(n: usize) -> Option<String> {
    (n > 1).then(|| format!("gs{}", n.min(260)))
}

/// A cell spanning `n` rows (`gr2`, which also drops a fixed height so it stretches); `None` for 1.
pub fn row_span(n: usize) -> Option<String> {
    (n > 1).then(|| format!("gr{}", n.min(500)))
}

/// Zoom levels the views offer, in percent; `z100` has no rule.
pub const ZOOMS: [u32; 11] = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200];

/// Font family stacks. A Latin face lacks CJK glyphs, so a Latin stack followed by a CJK stack
/// shows Word's split (ascii font for Latin text, eastAsia font for CJK) through plain fallback.
const LATIN: [(&str, &str); 4] = [
    ("s", r#""Times New Roman",Tinos,Georgia"#),
    ("h", "Calibri,Carlito,Arial,Helvetica"),
    ("m", r#"Consolas,Menlo,"Courier New""#),
    ("imp", r#"Impact,"Arial Black""#),
];
const CJK: [(&str, &str, &str); 5] = [
    ("song", r#""Songti SC",STSong,SimSun,"Noto Serif CJK SC""#, "serif"),
    ("hei", r#""Heiti SC",STHeiti,SimHei,"PingFang SC""#, "sans-serif"),
    ("kai", r#""Kaiti SC",STKaiti,KaiTi"#, "serif"),
    ("fang", r#"STFangsong,FangSong,"Songti SC""#, "serif"),
    ("yh", r#""PingFang SC","Microsoft YaHei","Hiragino Sans GB""#, "sans-serif"),
];

/// A font name's family: `(true, "song")` for a CJK face, `(false, "h")` for a Latin one.
fn family(name: &str) -> Option<(bool, &'static str)> {
    let n = name.trim().to_ascii_lowercase();
    if n.is_empty() {
        return None;
    }
    const RULES: [(&[&str], bool, &str); 10] = [
        (&["仿宋", "fangsong", "fang song"], true, "fang"),
        (&["雅黑", "yahei", "pingfang", "等线", "dengxian", "思源黑", "source han sans", "noto sans cjk", "hiragino sans"], true, "yh"),
        (&["宋", "song", "simsun", "nsimsun", "mincho", "明朝", "batang", "思源宋", "source han serif", "noto serif cjk"], true, "song"),
        (&["黑", "hei", "simhei", "gothic", "ゴシック", "dotum", "gulim"], true, "hei"),
        (&["楷", "kai"], true, "kai"),
        (&["impact", "arial black"], false, "imp"),
        (&["courier", "consolas", "mono", "menlo", "monaco", "lucida console", "code"], false, "m"),
        // "… Sans Serif" is a sans face: decide sans before serif.
        (&["sans", "arial", "helvetica", "calibri", "verdana", "tahoma", "segoe", "aptos", "trebuchet", "candara", "corbel", "gill"], false, "h"),
        (&["times", "georgia", "cambria", "garamond", "book antiqua", "palatino", "serif", "minion", "constantia"], false, "s"),
        (&["roman"], false, "s"),
    ];
    RULES.iter().find(|(keys, _, _)| keys.iter().any(|k| n.contains(k))).map(|(_, cjk, f)| (*cjk, *f))
}

/// The family class for one font name (`ffh`, `ffsong`), if the sheet has a stack for it.
pub fn font_class(name: &str) -> Option<String> {
    family(name).map(|(_, f)| format!("ff{f}"))
}

/// The family class for a Latin face plus an East Asian face (`ffs-song`); either may be empty.
/// A CJK face named as the Latin font wins (its own Latin glyphs are what Word shows).
pub fn font_pair(latin: &str, east_asia: &str) -> Option<String> {
    let l = family(latin);
    let e = family(east_asia).filter(|(cjk, _)| *cjk);
    match (l, e) {
        (Some((false, lf)), Some((_, ef))) => Some(format!("ff{lf}-{ef}")),
        (Some((_, f)), _) => Some(format!("ff{f}")),
        (None, Some((_, f))) => Some(format!("ff{f}")),
        (None, None) => None,
    }
}

// ---- the sheet ----------------------------------------------------------------------------

/// The whole style sheet: base rules, then every utility class.
pub fn stylesheet() -> String {
    let mut s = String::with_capacity(200 * 1024);
    s.push_str(BASE);
    s.push('\n');
    let o = ".opv .";
    let hex = |r: u8, g: u8, b: u8| format!("#{r:02x}{g:02x}{b:02x}");
    for (r, &rv) in C8.iter().enumerate() {
        for (g, &gv) in C8.iter().enumerate() {
            for (b, &bv) in C8.iter().enumerate() {
                let _ = writeln!(s, "{o}c{r}{g}{b}{{color:{}}}", hex(rv, gv, bv));
            }
        }
    }
    for (r, &rv) in C8.iter().enumerate() {
        for (g, &gv) in C8.iter().enumerate() {
            for (b, &bv) in C8.iter().enumerate() {
                let _ = writeln!(s, "{o}k{r}{g}{b}{{background-color:{}}}", hex(rv, gv, bv));
            }
        }
    }
    let sides = [("t", "top"), ("r", "right"), ("b", "bottom"), ("l", "left")];
    for (l, name) in sides {
        for line in Line::ALL {
            let _ = writeln!(s, "{o}b{l}{}{{border-{name}:{} #000}}", line.code(), line.css());
        }
        let _ = writeln!(s, "{o}b{l}n{{border-{name}:none}}");
    }
    for (l, name) in sides {
        for (r, &rv) in C4.iter().enumerate() {
            for (g, &gv) in C4.iter().enumerate() {
                for (b, &bv) in C4.iter().enumerate() {
                    let _ = writeln!(s, "{o}bc{l}{r}{g}{b}{{border-{name}-color:{}}}", hex(rv, gv, bv));
                }
            }
        }
    }
    for n in 2..=200u32 {
        let _ = writeln!(s, "{o}f{n}{{font-size:{}px}}", fmt(n as f64 * 2.0 / 3.0));
    }
    for n in 1..=600u32 {
        let px = n * 4;
        let _ = writeln!(s, "{o}w{n}{{width:{px}px;min-width:{px}px;max-width:{px}px}}");
    }
    for n in 1..=600u32 {
        let _ = writeln!(s, "{o}mw{n}{{max-width:{}px}}", n * 4);
    }
    for n in 1..=100u32 {
        let _ = writeln!(s, "{o}mt{n}{{margin-top:{}px}}", n * 2);
        let _ = writeln!(s, "{o}mb{n}{{margin-bottom:{}px}}", n * 2);
    }
    for n in 1..=200u32 {
        let _ = writeln!(s, "{o}pl{n}{{padding-left:{}px}}", n * 4);
        let _ = writeln!(s, "{o}pr{n}{{padding-right:{}px}}", n * 4);
    }
    for n in 1..=50u32 {
        let _ = writeln!(s, "{o}pt{n}{{padding-top:{}px}}", n * 4);
        let _ = writeln!(s, "{o}pb{n}{{padding-bottom:{}px}}", n * 4);
    }
    for n in 1..=100u32 {
        let _ = writeln!(s, "{o}ml{n}{{margin-left:-{}px}}", n * 4);
    }
    for n in 10..=80u32 {
        let _ = writeln!(s, "{o}lh{n}{{line-height:{}}}", fmt(n as f64 * 0.05));
    }
    for n in 6..=160u32 {
        let _ = writeln!(s, "{o}lp{n}{{line-height:{n}px}}");
    }
    for n in 1..=300u32 {
        let _ = writeln!(s, "{o}h{n}{{height:{}px}}", n * 2);
    }
    for n in 1..=15u32 {
        let _ = writeln!(s, "{o}xi{n}{{padding-left:{}px}}", n * 9 + 3);
    }
    // Tern's table layout ignores colspan/rowspan, so tables are CSS grids: a column count
    // per grid, and span classes per merged cell.
    for n in 1..=260u32 {
        let _ = writeln!(s, "{o}gt{n}{{grid-template-columns:repeat({n},max-content)}}");
    }
    for n in 2..=260u32 {
        let _ = writeln!(s, "{o}gs{n}{{grid-column:span {n}}}");
    }
    for n in 2..=500u32 {
        let _ = writeln!(s, "{o}gr{n}{{grid-row:span {n};height:auto}}");
    }
    for z in ZOOMS {
        if z != 100 {
            let _ = writeln!(s, "{o}z{z}{{zoom:{}}}", fmt(z as f64 / 100.0));
        }
    }
    s.push_str(UTIL);
    for (l, stack) in LATIN {
        let tail = match l {
            "s" => r#""Songti SC",SimSun,serif"#,
            "m" => r#""PingFang SC",monospace"#,
            _ => r#""PingFang SC","Microsoft YaHei",sans-serif"#,
        };
        let _ = writeln!(s, "{o}ff{l}{{font-family:{stack},{tail}}}");
    }
    for (c, stack, generic) in CJK {
        let _ = writeln!(s, "{o}ff{c}{{font-family:{stack},{generic}}}");
    }
    for (l, lstack) in LATIN {
        for (c, cstack, generic) in CJK {
            let _ = writeln!(s, "{o}ff{l}-{c}{{font-family:{lstack},{cstack},{generic}}}");
        }
    }
    s
}

fn fmt(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

/// Layout and tag rules. Specificity stays ≤ (0,2,0) so utilities override them.
const BASE: &str = r#"/* office-preview: generated by `office-preview css`; edit native/src/css.rs, not this file. */
.sf-main[data-surface='plugin.office-preview.doc']{padding:0;gap:0;overflow:hidden}
.opv{flex:1 1 0;min-height:0;overflow:auto;color:#000;background:light-dark(#e8e8ec,#232428);font-family:Calibri,Carlito,"PingFang SC","Microsoft YaHei",sans-serif;font-size:14.67px;line-height:1.2;white-space:normal}
.opv.opd{padding:16px 12px 24px}
.opv .pg{background:#fff;color:#000;margin:0 auto;padding:40px 48px;box-shadow:0 1px 4px rgb(0 0 0/28%);white-space:pre-wrap;overflow-wrap:break-word;tab-size:48px;line-height:1.15}
.opv .pg+.pg{margin-top:16px}
.opv p{margin:0;min-height:1em}
.opv h1,.opv h2,.opv h3,.opv h4{margin:0;font-size:inherit;font-weight:inherit}
.opv .sf-image{display:inline-block;vertical-align:bottom;max-width:100%}
.opv .sf-math{display:inline-block;vertical-align:middle}
.opv .sf-math.display{display:block}
.opv .in{display:inline-block}
.opv .lbl{display:inline-block;white-space:pre}
.opv .ph{display:inline-block;border:1px dashed #999;color:#666;padding:2px 8px;margin:2px 0;font-size:12px;background:#fafafa;white-space:normal}
.opv .tbx{border:1px solid #9a9a9a;padding:6px 8px;margin:4px 0}
.opv .hf{color:#444;font-size:.92em}
.opv .hfr{border-top:1px dashed #c8c8c8;margin:10px 0}
.opv .notes{border-top:1px solid #9a9a9a;margin-top:18px;padding-top:6px;font-size:.9em}
.opv .lnk{color:#0563c1;text-decoration-line:underline}
.opv .brk{display:block;height:0}
.opv .pbr{border-top:1px dashed #b0b0b0;margin:12px -48px;height:0}
.opv .xg{display:grid;width:max-content;background:#fff;color:#000;font-family:Calibri,Carlito,"PingFang SC","Microsoft YaHei",sans-serif;font-size:14.67px;line-height:1.25}
.xg>div{display:flex;flex-direction:column;justify-content:flex-end;box-sizing:border-box;min-width:0;height:20px;padding:1px 3px;white-space:pre;overflow:hidden;border-right:1px solid #e1e1e1;border-bottom:1px solid #e1e1e1}
.xg>.xh,.xg>.xr,.xg>.xc{background:#f2f2f2;color:#5b5b5b;font-size:12px;font-weight:normal;justify-content:center;text-align:center;border-right-color:#cfcfcf;border-bottom-color:#cfcfcf;white-space:nowrap}
.xg>.xh{position:sticky;top:0;z-index:3}
.xg>.xr{position:sticky;left:0;z-index:2;min-width:36px}
.xg>.xc{position:sticky;top:0;left:0;z-index:4}
.xg>.vt{justify-content:flex-start}
.xg>.vm{justify-content:center}
.xg>.vb{justify-content:flex-end}
.xg>.fz{border-bottom:2px solid #a6a6a6}
.opxng>div{border-right-color:transparent;border-bottom-color:transparent}
.xg>.sel{box-shadow:inset 0 0 0 2px #217346;z-index:1}
.xg>.hl{box-shadow:inset 0 0 0 9999px rgb(255 166 0/16%)}
.xg>.sl{box-shadow:inset 0 0 0 9999px rgb(33 115 70/11%)}
.xg>.sel.hl{box-shadow:inset 0 0 0 2px #217346,inset 0 0 0 9999px rgb(255 166 0/16%)}
.xg>.sel.sl{box-shadow:inset 0 0 0 2px #217346,inset 0 0 0 9999px rgb(33 115 70/11%)}
.xg>.hh{background:#d3e9db;color:#1b6b3f;font-weight:600}
.opv .dg{display:grid;width:max-content}
.dg>div{display:flex;flex-direction:column;justify-content:flex-start;box-sizing:border-box;min-width:0;padding:0 7px}
.dg>.vm{justify-content:center}
.dg>.vb{justify-content:flex-end}
.opv .xs{color:light-dark(#555,#aaa);font-size:12px;margin:10px 0 4px}
.opv .xm{background:#fff;padding:8px;display:inline-block;margin:6px 8px 0 0;box-shadow:0 1px 3px rgb(0 0 0/20%)}
.opv .xmsg{color:light-dark(#555,#aaa);font-size:12.5px;margin:10px 2px}
.opv.opx{padding:8px 8px 24px}
.opv .tjc{margin-left:auto;margin-right:auto}
.opv .tjr{margin-left:auto}
"#;

/// Text utilities: come last so they beat the color/size families where both apply.
const UTIL: &str = r#".opv .b{font-weight:bold}
.opv .nb{font-weight:normal}
.opv .i{font-style:italic}
.opv .u{text-decoration-line:underline}
.opv .uu{text-decoration-line:underline;text-decoration-style:double}
.opv .ud{text-decoration-line:underline;text-decoration-style:dotted}
.opv .uh{text-decoration-line:underline;text-decoration-style:dashed}
.opv .uw{text-decoration-line:underline;text-decoration-style:wavy}
.opv .s{text-decoration-line:line-through}
.opv .us{text-decoration-line:underline line-through}
.opv .sup{vertical-align:super;font-size:.66em}
.opv .sub{vertical-align:sub;font-size:.66em}
.opv .cap{text-transform:uppercase}
.opv .scap{text-transform:uppercase;font-size:.82em}
.opv .al{text-align:left}
.opv .ac{text-align:center}
.opv .ar{text-align:right}
.opv .aj{text-align:justify}
.opv .vt{vertical-align:top}
.opv .vm{vertical-align:middle}
.opv .vb{vertical-align:bottom}
.opv .ww{white-space:pre-wrap;overflow-wrap:break-word}
.opv .nw{white-space:pre}
.opv .ni{font-style:normal}
.opv .nu{text-decoration-line:none}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn office_css_is_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../office.css");
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            on_disk == stylesheet(),
            "office.css is stale: run `cargo run -q -- css > ../office.css` in native/"
        );
    }

    #[test]
    fn sheet_fits_the_manifest_cap() {
        assert!(stylesheet().len() < 256 * 1024, "{} bytes", stylesheet().len());
    }

    #[test]
    fn class_names() {
        assert_eq!(fg(Rgb(255, 0, 0)), "c700");
        assert_eq!(bg(Rgb(0xD9, 0xE1, 0xF2)), "k667");
        assert_eq!(size(22.0).as_deref(), Some("f22"));
        assert_eq!(width(64.0).as_deref(), Some("w16"));
        let mut s = String::new();
        border(&mut s, Side::Left, Border { line: Line::Thin, color: Some(Rgb(0x80, 0x80, 0x80)) });
        assert_eq!(s, " bl1 bcl222");
        assert_eq!(font_class("宋体").as_deref(), Some("ffsong"));
        assert_eq!(font_class("仿宋_GB2312").as_deref(), Some("fffang"));
        assert_eq!(font_class("Microsoft Sans Serif").as_deref(), Some("ffh"));
        assert_eq!(font_class("Arial Black").as_deref(), Some("ffimp"));
        assert_eq!(font_class("Times New Roman").as_deref(), Some("ffs"));
        assert_eq!(font_class("Noto Sans CJK SC").as_deref(), Some("ffyh"));
        // Word's per-script faces: a Latin face falls back to the East Asian one for CJK.
        assert_eq!(font_pair("Times New Roman", "宋体").as_deref(), Some("ffs-song"));
        assert_eq!(font_pair("Calibri", "").as_deref(), Some("ffh"));
        assert_eq!(font_pair("", "微软雅黑").as_deref(), Some("ffyh"));
        assert_eq!(font_pair("黑体", "宋体").as_deref(), Some("ffhei"));
        assert_eq!(font_pair("Calibri", "Arial").as_deref(), Some("ffh"));
        let sheet = stylesheet();
        assert!(sheet.contains(".opv .ffs-song{font-family:\"Times New Roman\""));
    }
}
