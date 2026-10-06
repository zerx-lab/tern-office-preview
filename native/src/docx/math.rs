//! OMML (`m:oMath`) → TeX for the `math` node.

use crate::xml::{self, attr, child, elems, is};
use roxmltree::Node;

/// TeX for one `m:oMath` (or any OMML container).
pub fn omml_to_tex(n: Node) -> String {
    let mut c = Conv { eqarr: 0 };
    let mut s = String::new();
    c.children(n, &mut s);
    tidy(&s)
}

struct Conv {
    /// Depth of `m:eqArr`, where `&` is an alignment point and not a literal.
    eqarr: u32,
}

/// Collapses runs of spaces and trims.
fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut sp = false;
    for c in s.trim().chars() {
        if c == ' ' {
            if !sp {
                out.push(' ');
            }
            sp = true;
        } else {
            sp = false;
            out.push(c);
        }
    }
    out
}

fn sym(c: char) -> Option<&'static str> {
    Some(match c {
        'α' => "\\alpha ",
        'β' => "\\beta ",
        'γ' => "\\gamma ",
        'δ' => "\\delta ",
        'ε' | 'ϵ' => "\\epsilon ",
        'ζ' => "\\zeta ",
        'η' => "\\eta ",
        'θ' | 'ϑ' => "\\theta ",
        'ι' => "\\iota ",
        'κ' => "\\kappa ",
        'λ' => "\\lambda ",
        'μ' | 'µ' => "\\mu ",
        'ν' => "\\nu ",
        'ξ' => "\\xi ",
        'π' => "\\pi ",
        'ρ' => "\\rho ",
        'σ' => "\\sigma ",
        'ς' => "\\varsigma ",
        'τ' => "\\tau ",
        'υ' => "\\upsilon ",
        'φ' | 'ϕ' => "\\phi ",
        'χ' => "\\chi ",
        'ψ' => "\\psi ",
        'ω' => "\\omega ",
        'Γ' => "\\Gamma ",
        'Δ' => "\\Delta ",
        'Θ' => "\\Theta ",
        'Λ' => "\\Lambda ",
        'Ξ' => "\\Xi ",
        'Π' => "\\Pi ",
        'Σ' => "\\Sigma ",
        'Φ' => "\\Phi ",
        'Ψ' => "\\Psi ",
        'Ω' => "\\Omega ",
        '×' => "\\times ",
        '÷' => "\\div ",
        '±' => "\\pm ",
        '∓' => "\\mp ",
        '≤' | '⩽' => "\\le ",
        '≥' | '⩾' => "\\ge ",
        '≠' => "\\ne ",
        '≈' => "\\approx ",
        '≡' => "\\equiv ",
        '≅' => "\\cong ",
        '∼' => "\\sim ",
        '∝' => "\\propto ",
        '≪' => "\\ll ",
        '≫' => "\\gg ",
        '∞' => "\\infty ",
        '∑' => "\\sum ",
        '∏' => "\\prod ",
        '∫' => "\\int ",
        '∬' => "\\iint ",
        '∭' => "\\iiint ",
        '∮' => "\\oint ",
        '√' => "\\surd ",
        '∂' => "\\partial ",
        '∇' => "\\nabla ",
        '∈' => "\\in ",
        '∉' => "\\notin ",
        '∋' => "\\ni ",
        '⊂' => "\\subset ",
        '⊃' => "\\supset ",
        '⊆' => "\\subseteq ",
        '⊇' => "\\supseteq ",
        '∪' => "\\cup ",
        '∩' => "\\cap ",
        '∅' => "\\emptyset ",
        '→' => "\\to ",
        '←' => "\\leftarrow ",
        '↔' => "\\leftrightarrow ",
        '⇒' => "\\Rightarrow ",
        '⇐' => "\\Leftarrow ",
        '⇔' => "\\Leftrightarrow ",
        '↑' => "\\uparrow ",
        '↓' => "\\downarrow ",
        '∀' => "\\forall ",
        '∃' => "\\exists ",
        '¬' => "\\neg ",
        '∧' => "\\wedge ",
        '∨' => "\\vee ",
        '·' | '⋅' | '∙' => "\\cdot ",
        '∘' => "\\circ ",
        '∗' => "* ",
        '−' => "-",
        '…' => "\\ldots ",
        '⋯' => "\\cdots ",
        '⋮' => "\\vdots ",
        '⋱' => "\\ddots ",
        '°' => "^\\circ ",
        '′' => "' ",
        '″' => "'' ",
        '⊥' => "\\perp ",
        '∥' => "\\parallel ",
        '∠' => "\\angle ",
        'ℏ' => "\\hbar ",
        'ℓ' => "\\ell ",
        'ℝ' => "\\mathbb{R} ",
        'ℕ' => "\\mathbb{N} ",
        'ℤ' => "\\mathbb{Z} ",
        'ℚ' => "\\mathbb{Q} ",
        'ℂ' => "\\mathbb{C} ",
        '⊕' => "\\oplus ",
        '⊗' => "\\otimes ",
        '△' => "\\triangle ",
        '∴' => "\\therefore ",
        '∵' => "\\because ",
        '\u{2061}' | '\u{2062}' | '\u{2063}' | '\u{2064}' => "",
        '\u{00A0}' | '\u{2009}' | '\u{2002}' | '\u{2003}' => "\\ ",
        _ => return None,
    })
}

fn push_text(s: &mut String, t: &str, eqarr: bool) {
    for c in t.chars() {
        if let Some(m) = sym(c) {
            s.push_str(m);
            continue;
        }
        match c {
            '#' | '$' | '%' | '_' | '{' | '}' => {
                s.push('\\');
                s.push(c);
            }
            '&' if eqarr => s.push('&'),
            '&' => s.push_str("\\&"),
            '~' => s.push_str("\\sim "),
            '^' => s.push_str("\\wedge "),
            '\\' => s.push_str("\\backslash "),
            c if c.is_whitespace() => {}
            c => s.push(c),
        }
    }
}

/// A delimiter character as `\left` / `\right` wants it.
fn delim(c: &str) -> String {
    match c {
        "" => ".".into(),
        "{" => "\\{".into(),
        "}" => "\\}".into(),
        "⟨" | "〈" | "<" => "\\langle ".into(),
        "⟩" | "〉" | ">" => "\\rangle ".into(),
        "‖" | "∥" => "\\| ".into(),
        "⌈" => "\\lceil ".into(),
        "⌉" => "\\rceil ".into(),
        "⌊" => "\\lfloor ".into(),
        "⌋" => "\\rfloor ".into(),
        "|" => "|".into(),
        "(" | ")" | "[" | "]" | "/" => c.into(),
        other => {
            let mut s = String::new();
            push_text(&mut s, other, false);
            s
        }
    }
}

fn prop_val<'a>(n: Node<'a, '_>, pr: &str, name: &str) -> Option<&'a str> {
    child(n, "m", pr).and_then(|p| child(p, "m", name)).and_then(|c| attr(c, "m", "val"))
}

fn flag(n: Node, pr: &str, name: &str) -> bool {
    match prop_val(n, pr, name) {
        Some(v) => xml::on_off(Some(v)),
        None => child(n, "m", pr).and_then(|p| child(p, "m", name)).is_some(),
    }
}

const FUNCS: [&str; 30] = [
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "coth", "log",
    "ln", "lg", "exp", "lim", "max", "min", "sup", "inf", "det", "dim", "gcd", "deg", "hom", "ker", "arg", "Pr",
];

impl Conv {
    fn children(&mut self, n: Node, s: &mut String) {
        for c in elems(n) {
            self.node(c, s);
        }
    }

    /// TeX of the child `m:name` of `n`, braced.
    fn arg(&mut self, n: Node, name: &str) -> String {
        let mut s = String::new();
        if let Some(c) = child(n, "m", name) {
            self.children(c, &mut s);
        }
        let t = tidy(&s);
        if t.is_empty() {
            "{}".into()
        } else {
            format!("{{{t}}}")
        }
    }

    fn raw(&mut self, n: Node, name: &str) -> String {
        let mut s = String::new();
        if let Some(c) = child(n, "m", name) {
            self.children(c, &mut s);
        }
        tidy(&s)
    }

    fn node(&mut self, n: Node, s: &mut String) {
        if xml::prefix(n.tag_name().namespace()) != "m" {
            // `mc:AlternateContent` and friends: take the first branch.
            if is(n, "mc", "AlternateContent") {
                if let Some(c) = elems(n).next() {
                    self.children(c, s);
                }
            }
            return;
        }
        match n.tag_name().name() {
            "r" => {
                let mut t = String::new();
                for c in elems(n) {
                    if is(c, "m", "t") {
                        t.push_str(&xml::text(c));
                    }
                }
                let rpr = child(n, "m", "rPr");
                let sty = rpr.and_then(|r| child(r, "m", "sty")).and_then(|c| attr(c, "m", "val"));
                let nor = rpr.and_then(|r| child(r, "m", "nor")).is_some_and(|c| xml::on_off(attr(c, "m", "val")));
                if nor {
                    s.push_str(&format!("\\text{{{}}}", t.replace('\\', "").replace(['{', '}'], "")));
                } else if matches!(sty, Some("p")) && t.chars().count() > 1 && t.chars().all(|c| c.is_ascii_alphabetic()) {
                    s.push_str(&format!("\\mathrm{{{t}}} "));
                } else if matches!(sty, Some("b") | Some("bi")) && t.chars().all(|c| c.is_ascii_alphanumeric()) && !t.is_empty() {
                    s.push_str(&format!("\\mathbf{{{t}}} "));
                } else {
                    push_text(s, &t, self.eqarr > 0);
                }
            }
            "f" => {
                let ty = prop_val(n, "fPr", "type").unwrap_or("bar");
                let (num, den) = (self.arg(n, "num"), self.arg(n, "den"));
                match ty {
                    "lin" | "skw" => s.push_str(&format!("{num}/{den} ")),
                    "noBar" => s.push_str(&format!("\\binom{num}{den} ")),
                    _ => s.push_str(&format!("\\frac{num}{den} ")),
                }
            }
            "sSup" => {
                let (e, sup) = (self.arg(n, "e"), self.arg(n, "sup"));
                s.push_str(&format!("{e}^{sup} "));
            }
            "sSub" => {
                let (e, sub) = (self.arg(n, "e"), self.arg(n, "sub"));
                s.push_str(&format!("{e}_{sub} "));
            }
            "sSubSup" => {
                let (e, sub, sup) = (self.arg(n, "e"), self.arg(n, "sub"), self.arg(n, "sup"));
                s.push_str(&format!("{e}_{sub}^{sup} "));
            }
            "sPre" => {
                let (e, sub, sup) = (self.arg(n, "e"), self.arg(n, "sub"), self.arg(n, "sup"));
                s.push_str(&format!("{{}}_{sub}^{sup}{e} "));
            }
            "rad" => {
                let e = self.arg(n, "e");
                let deg = self.raw(n, "deg");
                if flag(n, "radPr", "degHide") || deg.is_empty() {
                    s.push_str(&format!("\\sqrt{e} "));
                } else {
                    s.push_str(&format!("\\sqrt[{deg}]{e} "));
                }
            }
            "nary" => {
                let chr = prop_val(n, "naryPr", "chr").unwrap_or("∫");
                let op = match chr {
                    "∑" => "\\sum",
                    "∏" => "\\prod",
                    "∐" => "\\coprod",
                    "∫" => "\\int",
                    "∬" => "\\iint",
                    "∭" => "\\iiint",
                    "∮" => "\\oint",
                    "⋃" => "\\bigcup",
                    "⋂" => "\\bigcap",
                    "⋁" => "\\bigvee",
                    "⋀" => "\\bigwedge",
                    "" => "",
                    other => {
                        let mut t = String::new();
                        push_text(&mut t, other, false);
                        s.push_str(t.trim());
                        ""
                    }
                };
                s.push_str(op);
                let under = match prop_val(n, "naryPr", "limLoc") {
                    Some("undOvr") => true,
                    Some("subSup") => false,
                    _ => !matches!(chr, "∫" | "∬" | "∭" | "∮" | ""),
                };
                if !op.is_empty() {
                    s.push_str(if under { "\\limits" } else { "\\nolimits" });
                }
                if !flag(n, "naryPr", "subHide") {
                    let sub = self.raw(n, "sub");
                    if !sub.is_empty() {
                        s.push_str(&format!("_{{{sub}}}"));
                    }
                }
                if !flag(n, "naryPr", "supHide") {
                    let sup = self.raw(n, "sup");
                    if !sup.is_empty() {
                        s.push_str(&format!("^{{{sup}}}"));
                    }
                }
                let e = self.raw(n, "e");
                s.push_str(&format!(" {{{e}}} "));
            }
            "d" => {
                let beg = prop_val(n, "dPr", "begChr").unwrap_or("(");
                let end = prop_val(n, "dPr", "endChr").unwrap_or(")");
                let sep = prop_val(n, "dPr", "sepChr").unwrap_or("|");
                let parts: Vec<String> = elems(n).filter(|c| is(*c, "m", "e")).map(|c| {
                    let mut t = String::new();
                    self.children(c, &mut t);
                    tidy(&t)
                }).collect();
                let mid = format!(" \\middle{} ", delim(sep));
                s.push_str(&format!("\\left{} {} \\right{} ", delim(beg), parts.join(&mid), delim(end)));
            }
            "func" => {
                let name = self.raw(n, "fName");
                let e = self.raw(n, "e");
                let plain: String = name.replace("\\mathrm{", "").replace('}', "").trim().to_string();
                if FUNCS.contains(&plain.as_str()) {
                    s.push_str(&format!("\\{plain} {e} "));
                } else if !plain.is_empty() && plain.chars().all(|c| c.is_ascii_alphabetic()) {
                    s.push_str(&format!("\\operatorname{{{plain}}} {e} "));
                } else {
                    s.push_str(&format!("{name} {e} "));
                }
            }
            "acc" => {
                let chr = prop_val(n, "accPr", "chr").unwrap_or("\u{0302}");
                let cmd = match chr {
                    "\u{0302}" | "^" => "\\hat",
                    "\u{0303}" | "~" => "\\tilde",
                    "\u{0307}" | "˙" => "\\dot",
                    "\u{0308}" | "¨" => "\\ddot",
                    "\u{0304}" | "\u{00AF}" | "‾" | "\u{0305}" => "\\bar",
                    "\u{20D7}" | "→" => "\\vec",
                    "\u{030C}" | "ˇ" => "\\check",
                    "\u{0306}" | "˘" => "\\breve",
                    "\u{0301}" | "´" => "\\acute",
                    "\u{0300}" | "`" => "\\grave",
                    _ => "\\hat",
                };
                let e = self.arg(n, "e");
                s.push_str(&format!("{cmd}{e} "));
            }
            "bar" => {
                let e = self.arg(n, "e");
                if prop_val(n, "barPr", "pos") == Some("top") {
                    s.push_str(&format!("\\overline{e} "));
                } else {
                    s.push_str(&format!("\\underline{e} "));
                }
            }
            "borderBox" => {
                let e = self.arg(n, "e");
                s.push_str(&format!("\\boxed{e} "));
            }
            "box" | "phant" => {
                let e = self.raw(n, "e");
                s.push_str(&format!("{e} "));
            }
            "eqArr" => {
                self.eqarr += 1;
                let rows: Vec<String> = elems(n).filter(|c| is(*c, "m", "e")).map(|c| {
                    let mut t = String::new();
                    self.children(c, &mut t);
                    tidy(&t)
                }).collect();
                self.eqarr -= 1;
                s.push_str(&format!("\\begin{{aligned}} {} \\end{{aligned}} ", rows.join(" \\\\ ")));
            }
            "m" => {
                let rows: Vec<String> = elems(n).filter(|c| is(*c, "m", "mr")).map(|r| {
                    let cells: Vec<String> = elems(r).filter(|c| is(*c, "m", "e")).map(|c| {
                        let mut t = String::new();
                        self.children(c, &mut t);
                        tidy(&t)
                    }).collect();
                    cells.join(" & ")
                }).collect();
                s.push_str(&format!("\\begin{{matrix}} {} \\end{{matrix}} ", rows.join(" \\\\ ")));
            }
            "limLow" => {
                let e = self.raw(n, "e");
                let lim = self.raw(n, "lim");
                let name = e.trim_start_matches('\\').trim();
                if FUNCS.contains(&name) {
                    s.push_str(&format!("\\{name}_{{{lim}}} "));
                } else {
                    s.push_str(&format!("\\underset{{{lim}}}{{{e}}} "));
                }
            }
            "limUpp" => {
                let e = self.raw(n, "e");
                let lim = self.raw(n, "lim");
                s.push_str(&format!("\\overset{{{lim}}}{{{e}}} "));
            }
            "groupChr" => {
                let chr = prop_val(n, "groupChrPr", "chr").unwrap_or("⏟");
                let pos = prop_val(n, "groupChrPr", "pos");
                let top = pos == Some("top") || chr == "⏞";
                let e = self.arg(n, "e");
                s.push_str(&format!("{}{e} ", if top { "\\overbrace" } else { "\\underbrace" }));
            }
            // Containers whose children are the content.
            "oMath" | "oMathPara" | "e" | "num" | "den" | "sub" | "sup" | "deg" | "lim" | "fName" | "mr" => {
                self.children(n, s)
            }
            // Properties and anything unknown contribute nothing.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tex(inner: &str) -> String {
        let x = format!(
            r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math">{inner}</m:oMath>"#
        );
        let doc = roxmltree::Document::parse(&x).unwrap();
        omml_to_tex(doc.root_element())
    }

    fn r(t: &str) -> String {
        format!("<m:r><m:t>{t}</m:t></m:r>")
    }

    #[test]
    fn fractions_and_scripts() {
        assert_eq!(tex(&format!("<m:f><m:num>{}</m:num><m:den>{}</m:den></m:f>", r("a+b"), r("2"))), "\\frac{a+b}{2}");
        assert_eq!(tex(&format!("<m:f><m:fPr><m:type m:val=\"lin\"/></m:fPr><m:num>{}</m:num><m:den>{}</m:den></m:f>", r("a"), r("b"))), "{a}/{b}");
        assert_eq!(tex(&format!("<m:sSup><m:e>{}</m:e><m:sup>{}</m:sup></m:sSup>", r("x"), r("2"))), "{x}^{2}");
        assert_eq!(
            tex(&format!("<m:sSubSup><m:e>{}</m:e><m:sub>{}</m:sub><m:sup>{}</m:sup></m:sSubSup>", r("x"), r("i"), r("2"))),
            "{x}_{i}^{2}"
        );
        assert_eq!(
            tex(&format!("<m:sPre><m:sub>{}</m:sub><m:sup>{}</m:sup><m:e>{}</m:e></m:sPre>", r("1"), r("2"), r("X"))),
            "{}_{1}^{2}{X}"
        );
    }

    #[test]
    fn radicals_and_nary() {
        assert_eq!(tex(&format!("<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e>{}</m:e></m:rad>", r("x"))), "\\sqrt{x}");
        assert_eq!(tex(&format!("<m:rad><m:deg>{}</m:deg><m:e>{}</m:e></m:rad>", r("3"), r("x"))), "\\sqrt[3]{x}");
        let sum = format!(
            "<m:nary><m:naryPr><m:chr m:val=\"∑\"/></m:naryPr><m:sub>{}</m:sub><m:sup>{}</m:sup><m:e>{}</m:e></m:nary>",
            r("i=1"),
            r("n"),
            r("x")
        );
        assert_eq!(tex(&sum), "\\sum\\limits_{i=1}^{n} {x}");
        let int = format!("<m:nary><m:naryPr/><m:sub>{}</m:sub><m:sup>{}</m:sup><m:e>{}</m:e></m:nary>", r("0"), r("1"), r("f"));
        assert_eq!(tex(&int), "\\int\\nolimits_{0}^{1} {f}");
    }

    #[test]
    fn delimiters_functions_accents() {
        assert_eq!(tex(&format!("<m:d><m:e>{}</m:e></m:d>", r("x"))), "\\left( x \\right)");
        assert_eq!(
            tex(&format!("<m:d><m:dPr><m:begChr m:val=\"{{\"/><m:endChr m:val=\"\"/></m:dPr><m:e>{}</m:e><m:e>{}</m:e></m:d>", r("a"), r("b"))),
            "\\left\\{ a \\middle| b \\right."
        );
        assert_eq!(tex(&format!("<m:func><m:fName>{}</m:fName><m:e>{}</m:e></m:func>", r("sin"), r("x"))), "\\sin x");
        assert_eq!(tex(&format!("<m:func><m:fName>{}</m:fName><m:e>{}</m:e></m:func>", r("foo"), r("x"))), "\\operatorname{foo} x");
        assert_eq!(tex(&format!("<m:acc><m:accPr><m:chr m:val=\"\u{0303}\"/></m:accPr><m:e>{}</m:e></m:acc>", r("x"))), "\\tilde{x}");
        assert_eq!(tex(&format!("<m:bar><m:barPr><m:pos m:val=\"top\"/></m:barPr><m:e>{}</m:e></m:bar>", r("x"))), "\\overline{x}");
    }

    #[test]
    fn matrices_arrays_limits_and_text() {
        let m = format!(
            "<m:m><m:mr><m:e>{}</m:e><m:e>{}</m:e></m:mr><m:mr><m:e>{}</m:e><m:e>{}</m:e></m:mr></m:m>",
            r("1"),
            r("0"),
            r("0"),
            r("1")
        );
        assert_eq!(tex(&m), "\\begin{matrix} 1 & 0 \\\\ 0 & 1 \\end{matrix}");
        assert_eq!(
            tex(&format!("<m:eqArr><m:e>{}</m:e><m:e>{}</m:e></m:eqArr>", r("a&amp;=b"), r("c&amp;=d"))),
            "\\begin{aligned} a&=b \\\\ c&=d \\end{aligned}"
        );
        assert_eq!(
            tex(&format!("<m:limLow><m:e>{}</m:e><m:lim>{}</m:lim></m:limLow>", r("lim"), r("x→0"))),
            "\\lim_{x\\to 0}"
        );
        assert_eq!(
            tex(&format!("<m:groupChr><m:e>{}</m:e></m:groupChr>", r("abc"))),
            "\\underbrace{abc}"
        );
        assert_eq!(tex(&r("α×β≤10% a_b")), "\\alpha \\times \\beta \\le 10\\%a\\_b");
    }
}
