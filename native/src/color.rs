//! Colors: hex/ARGB parsing, the document theme, Excel indexed colors, tints and DrawingML
//! color modifiers.

use crate::xml::{attr, child, elems, is, prefix};
use roxmltree::Node;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const BLACK: Rgb = Rgb(0, 0, 0);
    pub const WHITE: Rgb = Rgb(255, 255, 255);

    /// `RRGGBB` or `AARRGGBB` (alpha ignored); `None` for anything else, `auto` included.
    pub fn hex(s: &str) -> Option<Rgb> {
        let s = s.trim().trim_start_matches('#');
        let s = if s.len() == 8 { &s[2..] } else { s };
        if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(s, 16).ok()?;
        Some(Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    fn to_hsl(self) -> (f64, f64, f64) {
        let r = self.0 as f64 / 255.0;
        let g = self.1 as f64 / 255.0;
        let b = self.2 as f64 / 255.0;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        if (max - min).abs() < 1e-9 {
            return (0.0, 0.0, l);
        }
        let d = max - min;
        let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
        let h = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h / 6.0, s, l)
    }

    fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
        let l = l.clamp(0.0, 1.0);
        if s <= 0.0 {
            let v = (l * 255.0).round() as u8;
            return Rgb(v, v, v);
        }
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        let f = |mut t: f64| {
            if t < 0.0 {
                t += 1.0;
            }
            if t > 1.0 {
                t -= 1.0;
            }
            let v = if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            };
            (v * 255.0).round().clamp(0.0, 255.0) as u8
        };
        Rgb(f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
    }

    /// Excel's `tint` (−1…1): darkens toward black below 0, lightens toward white above.
    pub fn tint(self, tint: f64) -> Rgb {
        if tint == 0.0 {
            return self;
        }
        let (h, s, l) = self.to_hsl();
        let l = if tint < 0.0 { l * (1.0 + tint) } else { l * (1.0 - tint) + tint };
        Rgb::from_hsl(h, s, l)
    }

    /// Mixes toward `other` by `t` (0 keeps `self`).
    pub fn mix(self, other: Rgb, t: f64) -> Rgb {
        let m = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * t).round().clamp(0.0, 255.0) as u8;
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }
}

/// The document theme: its 12 colors and its major/minor fonts.
#[derive(Clone, Debug)]
pub struct Theme {
    /// dk1, lt1, dk2, lt2, accent1…6, hlink, folHlink.
    pub colors: [Rgb; 12],
    pub major_latin: String,
    pub minor_latin: String,
    pub major_ea: String,
    pub minor_ea: String,
}

impl Default for Theme {
    fn default() -> Theme {
        // Office 2013+ defaults.
        let c = |s| Rgb::hex(s).unwrap();
        Theme {
            colors: [
                c("000000"),
                c("FFFFFF"),
                c("44546A"),
                c("E7E6E6"),
                c("4472C4"),
                c("ED7D31"),
                c("A5A5A5"),
                c("FFC000"),
                c("5B9BD5"),
                c("70AD47"),
                c("0563C1"),
                c("954F72"),
            ],
            major_latin: "Calibri Light".into(),
            minor_latin: "Calibri".into(),
            major_ea: String::new(),
            minor_ea: String::new(),
        }
    }
}

const SCHEME: [&str; 12] =
    ["dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"];

impl Theme {
    pub fn parse(xml: &str) -> Theme {
        let mut t = Theme::default();
        let Ok(doc) = roxmltree::Document::parse_with_options(xml, crate::xml::OPTS) else { return t };
        for n in doc.descendants() {
            if is(n, "a", "clrScheme") {
                for c in elems(n) {
                    let Some(i) = SCHEME.iter().position(|s| *s == c.tag_name().name()) else { continue };
                    if let Some(rgb) = elems(c).next().and_then(|v| base_color(v, &t)) {
                        t.colors[i] = rgb;
                    }
                }
            } else if is(n, "a", "majorFont") || is(n, "a", "minorFont") {
                let major = n.tag_name().name() == "majorFont";
                let latin = child(n, "a", "latin").and_then(|l| l.attribute("typeface")).unwrap_or("").to_string();
                let mut ea = child(n, "a", "ea").and_then(|l| l.attribute("typeface")).unwrap_or("").to_string();
                if ea.is_empty() {
                    // The East Asian face Office picks for Simplified Chinese.
                    ea = elems(n)
                        .find(|f| is(*f, "a", "font") && f.attribute("script") == Some("Hans"))
                        .and_then(|f| f.attribute("typeface"))
                        .unwrap_or("")
                        .to_string();
                }
                if major {
                    t.major_latin = latin;
                    t.major_ea = ea;
                } else {
                    t.minor_latin = latin;
                    t.minor_ea = ea;
                }
            }
        }
        t
    }

    /// A scheme color by DrawingML name (`accent1`, `tx1`, `bg2`, `phClr` → none).
    pub fn scheme(&self, name: &str) -> Option<Rgb> {
        let i = match name {
            "dk1" | "tx1" | "dark1" | "text1" => 0,
            "lt1" | "bg1" | "light1" | "background1" => 1,
            "dk2" | "tx2" | "dark2" | "text2" => 2,
            "lt2" | "bg2" | "light2" | "background2" => 3,
            "accent1" => 4,
            "accent2" => 5,
            "accent3" => 6,
            "accent4" => 7,
            "accent5" => 8,
            "accent6" => 9,
            "hlink" | "hyperlink" => 10,
            "folHlink" | "followedHyperlink" => 11,
            _ => return None,
        };
        Some(self.colors[i])
    }

    /// SpreadsheetML `theme="n"`: Excel swaps the first two pairs (0 = lt1, 1 = dk1, 2 = lt2, 3 = dk2).
    pub fn excel(&self, i: usize) -> Option<Rgb> {
        let j = match i {
            0 => 1,
            1 => 0,
            2 => 3,
            3 => 2,
            n if n < 12 => n,
            _ => return None,
        };
        Some(self.colors[j])
    }
}

fn base_color(n: Node, theme: &Theme) -> Option<Rgb> {
    if prefix(n.tag_name().namespace()) != "a" {
        return None;
    }
    match n.tag_name().name() {
        "srgbClr" => n.attribute("val").and_then(Rgb::hex),
        "sysClr" => n.attribute("lastClr").and_then(Rgb::hex).or_else(|| match n.attribute("val") {
            Some("window") => Some(Rgb::WHITE),
            _ => Some(Rgb::BLACK),
        }),
        "schemeClr" => n.attribute("val").and_then(|v| theme.scheme(v)),
        "prstClr" => n.attribute("val").and_then(preset),
        "scrgbClr" => {
            let p = |k| attr(n, "", k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 100000.0;
            let g = |v: f64| (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8;
            Some(Rgb(g(p("r")), g(p("g")), g(p("b"))))
        }
        "hslClr" => {
            let p = |k| attr(n, "", k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
            Some(Rgb::from_hsl(p("hue") / 60000.0 / 360.0, p("sat") / 100000.0, p("lum") / 100000.0))
        }
        _ => None,
    }
}

fn preset(name: &str) -> Option<Rgb> {
    Some(match name {
        "black" => Rgb(0, 0, 0),
        "white" => Rgb(255, 255, 255),
        "red" => Rgb(255, 0, 0),
        "green" => Rgb(0, 128, 0),
        "blue" => Rgb(0, 0, 255),
        "yellow" => Rgb(255, 255, 0),
        "gray" | "grey" => Rgb(128, 128, 128),
        "lightGray" | "ltGray" => Rgb(211, 211, 211),
        "darkGray" | "dkGray" => Rgb(169, 169, 169),
        "orange" => Rgb(255, 165, 0),
        "purple" => Rgb(128, 0, 128),
        "cyan" | "aqua" => Rgb(0, 255, 255),
        "magenta" | "fuchsia" => Rgb(255, 0, 255),
        _ => return None,
    })
}

/// WordprocessingML highlight names.
pub fn highlight(name: &str) -> Option<Rgb> {
    Some(match name {
        "yellow" => Rgb(255, 255, 0),
        "green" => Rgb(0, 255, 0),
        "cyan" => Rgb(0, 255, 255),
        "magenta" => Rgb(255, 0, 255),
        "blue" => Rgb(0, 0, 255),
        "red" => Rgb(255, 0, 0),
        "darkBlue" => Rgb(0, 0, 139),
        "darkCyan" => Rgb(0, 139, 139),
        "darkGreen" => Rgb(0, 100, 0),
        "darkMagenta" => Rgb(139, 0, 139),
        "darkRed" => Rgb(139, 0, 0),
        "darkYellow" => Rgb(128, 128, 0),
        "darkGray" => Rgb(169, 169, 169),
        "lightGray" => Rgb(211, 211, 211),
        "black" => Rgb(0, 0, 0),
        "white" => Rgb(255, 255, 255),
        _ => return None,
    })
}

/// Excel's legacy indexed palette (0–63), 64 = system foreground, 65 = system background.
pub fn indexed(i: usize, custom: &[Rgb]) -> Option<Rgb> {
    if let Some(c) = custom.get(i) {
        return Some(*c);
    }
    const P: [u32; 64] = [
        0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x000000, 0xFFFFFF, 0xFF0000,
        0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000, 0x008000, 0x000080, 0x808000, 0x800080, 0x008080,
        0xC0C0C0, 0x808080, 0x9999FF, 0x993366, 0xFFFFCC, 0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080,
        0xFF00FF, 0xFFFF00, 0x00FFFF, 0x800080, 0x800000, 0x008080, 0x0000FF, 0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99,
        0x99CCFF, 0xFF99CC, 0xCC99FF, 0xFFCC99, 0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600, 0x666699,
        0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399, 0x333333,
    ];
    match i {
        0..=63 => {
            let v = P[i];
            Some(Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
        }
        64 => Some(Rgb::BLACK),
        65 => Some(Rgb::WHITE),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excel_tints() {
        // Accent1 4472C4 at +0.7999 is Excel's D9E1F2; float HSL lands within 2 of Excel's
        // integer HLS, far below the 8-level quantization step.
        let c = Rgb::hex("4472C4").unwrap().tint(0.7999816888943144);
        let near = |a: u8, b: u8| (a as i32 - b as i32).abs() <= 2;
        assert!(near(c.0, 0xD9) && near(c.1, 0xE1) && near(c.2, 0xF2), "{c:?}");
        let d = Rgb::hex("FFFFFF").unwrap().tint(-0.1499984740745262);
        assert_eq!(d, Rgb(0xD9, 0xD9, 0xD9));
    }

    #[test]
    fn hex_forms() {
        assert_eq!(Rgb::hex("FF00FF00"), Some(Rgb(0, 255, 0)));
        assert_eq!(Rgb::hex("auto"), None);
    }
}
