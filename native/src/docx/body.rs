//! Paragraphs, runs, fields, notes, headers and footers.

use super::drawing::Drawn;
use super::math::omml_to_tex;
use super::numbering::{bullet_char, Label, Suff};
use super::props::{cjk_counts, Jc, LineRule, PPr, RPr, UKind, Vert};
use super::{pbr, Ctx, Field, NoteRef, OutlineItem, Phase};
use crate::color::Rgb;
use crate::css::{self, Side};
use crate::node::Node;
use crate::xml::{self, attr, child, children, elems, is, qname};
use roxmltree::Node as X;

/// A piece of a paragraph's line content.
pub enum Item {
    Seg(Seg),
    Node(Node),
    PageBreak,
}

pub struct Seg {
    /// Size / font / color / weight of the text; `None` inherits everything from the `p`.
    pub key: Option<PClass>,
    /// Classes that do not depend on the paragraph (italic, underline, background, super…).
    pub extra: String,
    pub href: Option<String>,
    pub text: String,
}

/// What the `p` element itself carries, so runs only add what differs.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct PClass {
    pub bold: bool,
    pub color: Option<Rgb>,
    pub size: Option<f64>,
    pub font: Option<String>,
}

struct PState {
    items: Vec<Item>,
    sides: Vec<Node>,
    rpr: RPr,
    href: Option<String>,
    external: bool,
}

/// The family class for a run: its Latin (ascii) face with its East Asian face as fallback, the
/// way Word picks a face per script.
fn font_for(r: &RPr, _cjk: bool) -> Option<String> {
    css::font_pair(r.font_ascii.as_deref().unwrap_or(""), r.font_ea.as_deref().unwrap_or(""))
}

/// What the paragraph element carries for a base run property set.
pub fn pclass(r: &RPr, cjk: bool) -> PClass {
    PClass {
        bold: r.b == Some(true),
        color: r.color.flatten().filter(|c| *c != Rgb::BLACK),
        size: r.size,
        font: font_for(r, cjk),
    }
}

/// Classes that put `pc` on the paragraph element.
fn pc_classes(pc: &PClass) -> String {
    let mut s = String::new();
    key_diff(pc, &PClass::default(), &mut s);
    s
}

/// What the run says about the properties a `p` can carry for it.
pub fn run_key(r: &RPr, cjk: bool) -> PClass {
    PClass {
        bold: r.b == Some(true),
        color: r.color.flatten().filter(|c| *c != Rgb::BLACK),
        size: if matches!(r.vert, Some(Vert::Sup) | Some(Vert::Sub)) { None } else { r.size },
        font: font_for(r, cjk),
    }
}

/// Classes `key` needs on top of a paragraph carrying `pc`.
pub fn key_diff(key: &PClass, pc: &PClass, out: &mut String) {
    let mut add = |c: &str| {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(c);
    };
    if key.bold != pc.bold {
        add(if key.bold { "b" } else { "nb" });
    }
    if key.color != pc.color {
        add(&css::fg(key.color.unwrap_or(Rgb::BLACK)));
    }
    if key.size.is_some() && key.size != pc.size {
        if let Some(c) = key.size.and_then(css::size) {
            add(&c);
        }
    }
    if key.font.is_some() && key.font != pc.font {
        add(key.font.as_deref().unwrap_or(""));
    }
}

/// Run classes that do not depend on the paragraph.
pub fn run_extra(r: &RPr) -> String {
    let mut s = String::new();
    let mut add = |c: &str| {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(c);
    };
    if r.i == Some(true) {
        add("i");
    }
    let u = r.u.filter(|k| *k != UKind::None);
    let strike = r.strike == Some(true) || r.dstrike == Some(true);
    match (u, strike) {
        (Some(_), true) => add("us"),
        (Some(UKind::Double), false) => add("uu"),
        (Some(UKind::Dotted), false) => add("ud"),
        (Some(UKind::Dash), false) => add("uh"),
        (Some(UKind::Wave), false) => add("uw"),
        (Some(_), false) => add("u"),
        (None, true) => add("s"),
        (None, false) => {}
    }
    if let Some(c) = r.highlight.flatten().or_else(|| r.shd.flatten()) {
        add(&css::bg(c));
    }
    match r.vert {
        Some(Vert::Sup) => add("sup"),
        Some(Vert::Sub) => add("sub"),
        _ => {}
    }
    if r.caps == Some(true) {
        add("cap");
    } else if r.small_caps == Some(true) {
        add("scap");
    }
    s
}

/// The paragraph's class set from its dominant text (by character count).
fn dominant(items: &[Item]) -> Option<PClass> {
    let mut tally: Vec<(PClass, usize)> = Vec::new();
    for it in items {
        if let Item::Seg(Seg { key: Some(k), text, .. }) = it {
            let n = text.chars().count();
            match tally.iter_mut().find(|(c, _)| c == k) {
                Some((_, w)) => *w += n,
                None => tally.push((k.clone(), n)),
            }
        }
    }
    let mut best: Option<(PClass, usize)> = None;
    for (k, w) in tally {
        if best.as_ref().is_none_or(|(_, bw)| w > *bw) {
            best = Some((k, w));
        }
    }
    best.map(|(k, _)| k)
}

fn push_seg(items: &mut Vec<Item>, key: Option<PClass>, extra: String, href: Option<String>, text: String) {
    if text.is_empty() {
        return;
    }
    if let Some(Item::Seg(last)) = items.last_mut() {
        if last.key == key && last.extra == extra && last.href == href {
            last.text.push_str(&text);
            return;
        }
    }
    items.push(Item::Seg(Seg { key, extra, href, text }));
}

fn roman_lower(mut n: usize) -> String {
    let t = [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
    let mut s = String::new();
    for (v, r) in t {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

/// `HYPERLINK "url"` (without `\l`) → the url.
fn hyperlink_target(instr: &str) -> Option<String> {
    let t = instr.trim_start();
    let rest = t.strip_prefix("HYPERLINK").or_else(|| t.strip_prefix("hyperlink"))?;
    if rest.split_whitespace().any(|w| w == "\\l") {
        return None;
    }
    let s = rest.find('"')?;
    let e = rest[s + 1..].find('"')?;
    let url = &rest[s + 1..s + 1 + e];
    (url.contains("://") || url.starts_with("mailto:")).then(|| url.to_string())
}

impl Ctx<'_> {
    // ---- blocks ---------------------------------------------------------------------------

    /// One body-level (or cell-level) child element.
    pub fn block_child(&mut self, c: X, out: &mut Vec<Node>) {
        match qname(c).as_str() {
            "w:p" => self.paragraph(c, out),
            "w:tbl" => {
                let top = self.depth == 0 && self.tbl.is_none() && self.outline_on;
                let t = self.table(c, top);
                out.extend(t);
            }
            "w:sdt" => {
                if let Some(content) = child(c, "w", "sdtContent") {
                    self.blocks(content, out);
                }
            }
            "w:sdtContent" | "w:ins" | "w:moveTo" | "w:customXml" | "w:smartTag" | "w:txbxContent" => {
                self.blocks(c, out)
            }
            "mc:AlternateContent" => {
                if let Some(ch) = elems(c).find(|e| is(*e, "mc", "Choice")).or_else(|| elems(c).find(|e| is(*e, "mc", "Fallback"))) {
                    self.blocks(ch, out);
                }
            }
            _ => {}
        }
    }

    pub fn blocks(&mut self, root: X, out: &mut Vec<Node>) {
        for c in elems(root) {
            self.block_child(c, out);
        }
    }

    // ---- paragraphs -----------------------------------------------------------------------

    pub fn paragraph(&mut self, p: X, out: &mut Vec<Node>) {
        // A field whose instruction never closed must not swallow the rest of the document.
        self.fields.retain(|f| f.phase == Phase::Result);

        let ppr_node = child(p, "w", "pPr");
        let dppr = ppr_node.map(|n| PPr::parse(n, &self.theme)).unwrap_or_default();
        let style = dppr.style.clone();
        let sty = self.styles.para(style.as_deref());
        let mut rpr = self.styles.def_rpr.clone();
        let mut ppr = self.styles.def_ppr.clone();
        if let Some(t) = &self.tbl {
            ppr.overlay(&t.ppr);
            rpr.overlay(&t.rpr);
        }
        if let Some(s) = &sty {
            ppr.overlay(&s.0);
            rpr.overlay(&s.1);
        }
        let mark_rpr = ppr_node
            .and_then(|n| child(n, "w", "rPr"))
            .map(|n| RPr::parse(n, &self.theme))
            .unwrap_or_default();

        // Numbering: the paragraph's own numPr, else the style's.
        let num_id = dppr.num_id.or(ppr.num_id).filter(|n| *n != 0);
        let ilvl = dppr
            .ilvl
            .or_else(|| {
                if dppr.num_id.is_some() {
                    None
                } else {
                    ppr.ilvl.or_else(|| {
                        let sid = style.as_deref().or(self.styles.default_para());
                        sid.and_then(|s| self.numbering.level_for_style(num_id?, s))
                    })
                }
            })
            .unwrap_or(0);
        let label: Option<Label> = num_id.and_then(|n| self.numbering.label(n, ilvl));
        if let Some(l) = &label {
            ppr.overlay(&l.ppr);
        }
        ppr.overlay(&dppr);

        // Heading level for the outline.
        let level = match dppr.outline {
            Some(o) => (o < 9).then_some(o + 1),
            None => self.styles.heading(style.as_deref()),
        };

        // Script of the paragraph decides which font family class the `p` itself takes.
        let (cjk_n, all_n) = cjk_counts(&paragraph_text(p, 240));
        let cjk = all_n > 0 && cjk_n * 3 > all_n;

        let empty_mark = !p.descendants().any(|n| is(n, "w", "r") || is(n, "w", "hyperlink") || is(n, "w", "fldSimple") || is(n, "m", "oMath"));
        let mut p_rpr = rpr.clone();
        if empty_mark {
            p_rpr.overlay(&mark_rpr);
        }

        let mut st = PState { items: Vec::new(), sides: Vec::new(), rpr: rpr.clone(), href: None, external: false };
        if let Some((extra, text)) = self.prefix.take() {
            st.items.push(Item::Seg(Seg { key: None, extra, href: None, text }));
        }
        for c in elems(p) {
            if !is(c, "w", "pPr") {
                self.inline(c, &mut st);
            }
        }
        let pc = dominant(&st.items).unwrap_or_else(|| pclass(&p_rpr, cjk));
        let font_cls = pc_classes(&pc);

        // Paragraph element classes.
        let size_hp = pc.size.or(rpr.size).unwrap_or(21.0);
        let size_px = size_hp / 2.0 * 4.0 / 3.0;
        let tw_of = |tw: Option<f64>, chars: Option<f64>| tw.or(chars.map(|c| c / 100.0 * size_hp / 2.0 * 20.0));
        let max_indent = (self.avail * 0.7).max(60.0);
        let left_px = (tw_of(ppr.left, ppr.left_chars).unwrap_or(0.0) / 15.0).clamp(0.0, max_indent);
        let right_px = (ppr.right.unwrap_or(0.0) / 15.0).clamp(0.0, max_indent);
        let first_px = tw_of(ppr.first, ppr.first_chars).unwrap_or(0.0) / 15.0;

        let mut cls = String::new();
        let mut add = |c: Option<String>| {
            if let Some(c) = c {
                cls.push(' ');
                cls.push_str(&c);
            }
        };
        match ppr.jc {
            Some(Jc::Center) => add(Some("ac".into())),
            Some(Jc::Right) => add(Some("ar".into())),
            Some(Jc::Both) => add(Some("aj".into())),
            _ => {}
        }
        add(css::pad_left(left_px));
        add(css::pad_right(right_px));
        let same_style = |sib: Option<X>| -> bool {
            let Some(s) = sib.filter(|s| is(*s, "w", "p")) else { return false };
            let sid = child(s, "w", "pPr").and_then(|pp| child(pp, "w", "pStyle")).and_then(xml::wval);
            sid == style.as_deref()
        };
        let contextual = ppr.contextual == Some(true);
        let mut before = if ppr.before_auto == Some(true) {
            14.0
        } else {
            ppr.before.map(|t| t / 15.0).or(ppr.before_lines.map(|l| l / 100.0 * size_px * 1.2)).unwrap_or(0.0)
        };
        let mut after = if ppr.after_auto == Some(true) {
            14.0
        } else {
            ppr.after.map(|t| t / 15.0).or(ppr.after_lines.map(|l| l / 100.0 * size_px * 1.2)).unwrap_or(0.0)
        };
        if contextual {
            if same_style(p.prev_sibling_element()) {
                before = 0.0;
            }
            if same_style(p.next_sibling_element()) {
                after = 0.0;
            }
        }
        add(css::margin_top(before));
        add(css::margin_bottom(after));
        match (ppr.line, ppr.line_rule.unwrap_or(LineRule::Auto)) {
            (Some(l), LineRule::Auto) if (l - 240.0).abs() > 6.0 => add(css::line_mult(l / 240.0 * 1.15)),
            (Some(l), LineRule::Exact) => add(css::line_px(l / 15.0)),
            (Some(l), LineRule::AtLeast) => add(css::line_px((l / 15.0).max(size_px * 1.2))),
            _ => {}
        }
        if let Some(Some(c)) = ppr.shd {
            add(Some(css::bg(c)));
        }
        let mut bcls = String::new();
        for (side, b) in [
            (Side::Top, ppr.borders.top),
            (Side::Left, ppr.borders.left),
            (Side::Bottom, ppr.borders.bottom),
            (Side::Right, ppr.borders.right),
        ] {
            if let Some(b) = b.and_then(|b| b.border()) {
                css::border(&mut bcls, side, b);
            }
        }
        let p_cls = format!("{cls} {bcls} {font_cls}").split_whitespace().collect::<Vec<_>>().join(" ");

        // Leading nodes of the first line: first-line indent, hanging pull, list label.
        let mut lead: Vec<Node> = Vec::new();
        let hang = if first_px < -2.0 { -first_px } else { 0.0 };
        if let Some(l) = &label {
            let mut lrpr = rpr.clone();
            lrpr.overlay(&mark_rpr);
            lrpr.overlay(&l.rpr);
            let lcjk = cjk_counts(&l.text).0 > 0;
            let mut lc = String::new();
            key_diff(&run_key(&lrpr, lcjk), &pc, &mut lc);
            lc.push(' ');
            lc.push_str(&run_extra(&lrpr));
            let suffix = match l.suff {
                Suff::Tab => "\t",
                Suff::Space => " ",
                Suff::Nothing => "",
            };
            let lsize_px = lrpr.size.unwrap_or(size_hp) / 2.0 * 4.0 / 3.0;
            let est: f64 = l.text.chars().map(|c| if super::props::is_cjk(c) { lsize_px } else { lsize_px * 0.62 }).sum();
            let mut span_cls = format!("lbl {lc}");
            let text;
            if hang > 0.0 {
                if let Some(c) = css::pull_left(hang) {
                    span_cls.push(' ');
                    span_cls.push_str(&c);
                }
                if est + 2.0 <= hang && l.suff == Suff::Tab {
                    if let Some(c) = css::width(hang) {
                        span_cls.push(' ');
                        span_cls.push_str(&c);
                    }
                    text = l.text.clone();
                } else {
                    text = format!("{}{}", l.text, if l.suff == Suff::Nothing { "" } else { " " });
                }
            } else {
                if first_px >= 2.0 {
                    if let Some(c) = css::width(first_px) {
                        lead.push(Node::el("span").class(&format!("in {c}")));
                    }
                }
                text = format!("{}{}", l.text, suffix);
            }
            lead.push(Node::el("span").class(&span_cls.split_whitespace().collect::<Vec<_>>().join(" ")).text(text));
        } else if hang > 0.0 {
            if let Some(c) = css::pull_left(hang) {
                lead.push(Node::el("span").class(&format!("in {c}")));
            }
        } else if first_px >= 2.0 {
            if let Some(c) = css::width(first_px) {
                lead.push(Node::el("span").class(&format!("in {c}")));
            }
        }

        if ppr.page_break_before == Some(true) {
            out.push(pbr());
        }

        // Outline entry.
        if self.outline_on {
            if let Some(l) = level {
                let text: String = st
                    .items
                    .iter()
                    .filter_map(|i| if let Item::Seg(s) = i { Some(s.text.as_str()) } else { None })
                    .collect();
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !text.is_empty() {
                    let t: String = text.chars().take(120).collect();
                    self.outline.push(OutlineItem { text: t, level: l, block: self.block });
                    self.heading = l;
                }
            }
        }

        // Split at explicit page breaks: [p] pbr [p] …
        let mut groups: Vec<(Vec<Item>, bool)> = Vec::new();
        let mut cur: Vec<Item> = Vec::new();
        for it in st.items {
            if matches!(it, Item::PageBreak) {
                groups.push((std::mem::take(&mut cur), true));
            } else {
                cur.push(it);
            }
        }
        groups.push((cur, false));
        let n_groups = groups.len();
        let mut lead = Some(lead);
        for (gi, (items, brk)) in groups.into_iter().enumerate() {
            let first = gi == 0;
            let last = gi + 1 == n_groups;
            let has_lead = first && lead.as_ref().is_some_and(|l| !l.is_empty());
            if items.is_empty() && !has_lead && !first && last {
                break;
            }
            if items.is_empty() && !has_lead && brk && first {
                out.push(pbr());
                continue;
            }
            let node = build_p(&p_cls, &pc, if first { lead.take().unwrap_or_default() } else { Vec::new() }, items);
            out.push(node);
            if brk {
                out.push(pbr());
            }
        }
        out.extend(st.sides);
    }

    // ---- inline content -------------------------------------------------------------------

    fn inline_children(&mut self, n: X, st: &mut PState) {
        for c in elems(n) {
            self.inline(c, st);
        }
    }

    fn effective_href(&self, st: &PState) -> Option<String> {
        st.href.clone().or_else(|| self.fields.iter().rev().find_map(|f| f.href.clone()))
    }

    fn inline(&mut self, c: X, st: &mut PState) {
        match qname(c).as_str() {
            "w:r" => self.run(c, st),
            "w:hyperlink" => {
                let target = attr(c, "r", "id").and_then(|id| self.rels.get(id)).filter(|r| r.external).map(|r| r.target.clone());
                let saved = (st.href.take(), st.external);
                st.external = target.is_some();
                st.href = target;
                self.inline_children(c, st);
                st.href = saved.0;
                st.external = saved.1;
            }
            "w:fldSimple" => {
                let instr = attr(c, "w", "instr").unwrap_or("");
                self.fields.push(Field { phase: Phase::Result, instr: instr.to_string(), href: hyperlink_target(instr) });
                self.inline_children(c, st);
                self.fields.pop();
            }
            "w:ins" | "w:moveTo" | "w:smartTag" | "w:customXml" | "w:dir" | "w:bdo" | "w:sdtContent" => {
                self.inline_children(c, st)
            }
            "w:sdt" => {
                if let Some(content) = child(c, "w", "sdtContent") {
                    self.inline_children(content, st);
                }
            }
            "m:oMathPara" => {
                for m in children(c, "m", "oMath") {
                    self.math(m, true, st);
                }
            }
            "m:oMath" => self.math(c, false, st),
            "mc:AlternateContent" => {
                let before = st.items.len();
                let sides = st.sides.len();
                if let Some(ch) = elems(c).find(|e| is(*e, "mc", "Choice")) {
                    self.inline_children(ch, st);
                }
                if st.items.len() == before && st.sides.len() == sides {
                    if let Some(fb) = elems(c).find(|e| is(*e, "mc", "Fallback")) {
                        self.inline_children(fb, st);
                    }
                }
            }
            "w:drawing" | "w:pict" | "w:object" => {
                let mut d = Drawn::default();
                self.draw(c, &mut d);
                st.items.extend(d.inline.into_iter().map(Item::Node));
                st.sides.extend(d.sides);
            }
            _ => {}
        }
    }

    fn math(&mut self, m: X, display: bool, st: &mut PState) {
        let tex = omml_to_tex(m);
        if tex.is_empty() {
            return;
        }
        st.items.push(Item::Node(Node::kind("math").prop("text", tex).prop("display", display)));
    }

    fn hidden(&self, vanish: bool) -> bool {
        vanish || self.fields.iter().any(|f| f.phase == Phase::Instr)
    }

    fn flush(&mut self, st: &mut PState, rpr: &RPr, buf: &mut String) {
        if buf.is_empty() {
            return;
        }
        let text = std::mem::take(buf);
        let cjk = cjk_counts(&text).0 > 0;
        let mut extra = run_extra(rpr);
        let href = self.effective_href(st);
        if href.is_some() && st.external && rpr.color.is_none() {
            let off = if rpr.u == Some(UKind::None) { " nu" } else { "" };
            extra = format!("lnk{off} {extra}").trim().to_string();
        }
        push_seg(&mut st.items, Some(run_key(rpr, cjk)), extra, href, text);
    }

    fn run(&mut self, r: X, st: &mut PState) {
        let mut rpr = st.rpr.clone();
        if let Some(d) = child(r, "w", "rPr") {
            let d = RPr::parse(d, &self.theme);
            if let Some(c) = d.rstyle.as_deref().and_then(|s| self.styles.char(s)) {
                rpr.overlay(&c);
            }
            rpr.overlay(&d);
        }
        let vanish = rpr.vanish == Some(true);
        let mut buf = String::new();
        for c in elems(r) {
            match qname(c).as_str() {
                "w:t" => {
                    if !self.hidden(vanish) {
                        buf.push_str(&xml::text(c));
                    }
                }
                "w:tab" | "w:ptab" => {
                    if !self.hidden(vanish) {
                        buf.push('\t');
                    }
                }
                "w:br" => {
                    if !self.hidden(vanish) {
                        match attr(c, "w", "type") {
                            Some("page") => {
                                self.flush(st, &rpr, &mut buf);
                                st.items.push(Item::PageBreak);
                            }
                            Some("column") => {}
                            _ => buf.push('\n'),
                        }
                    }
                }
                "w:cr" => {
                    if !self.hidden(vanish) {
                        buf.push('\n');
                    }
                }
                "w:noBreakHyphen" => {
                    if !self.hidden(vanish) {
                        buf.push('\u{2011}');
                    }
                }
                "w:sym" => {
                    if !self.hidden(vanish) {
                        if let Some(ch) = attr(c, "w", "char").and_then(|h| u32::from_str_radix(h.trim_start_matches("0x"), 16).ok()).and_then(char::from_u32) {
                            buf.push_str(&bullet_char(&ch.to_string(), attr(c, "w", "font")));
                        }
                    }
                }
                "w:fldChar" => match attr(c, "w", "fldCharType") {
                    Some("begin") => {
                        self.flush(st, &rpr, &mut buf);
                        self.fields.push(Field { phase: Phase::Instr, instr: String::new(), href: None });
                    }
                    Some("separate") => {
                        self.flush(st, &rpr, &mut buf);
                        if let Some(f) = self.fields.last_mut() {
                            f.phase = Phase::Result;
                            f.href = hyperlink_target(&f.instr);
                        }
                    }
                    Some("end") => {
                        self.flush(st, &rpr, &mut buf);
                        self.fields.pop();
                    }
                    _ => {}
                },
                "w:instrText" => {
                    if let Some(f) = self.fields.last_mut() {
                        if f.phase == Phase::Instr {
                            f.instr.push_str(&xml::text(c));
                        }
                    }
                }
                "w:footnoteReference" | "w:endnoteReference" => {
                    if !self.hidden(vanish) {
                        self.flush(st, &rpr, &mut buf);
                        let endnote = c.tag_name().name() == "endnoteReference";
                        if let Some(id) = attr(c, "w", "id") {
                            let num = self.note_number(endnote, id);
                            let mut r2 = rpr.clone();
                            r2.vert = Some(Vert::Sup);
                            let mut b2 = num;
                            self.flush(st, &r2, &mut b2);
                        }
                    }
                }
                "w:drawing" | "w:pict" | "w:object" | "mc:AlternateContent" => {
                    if !self.hidden(vanish) {
                        self.flush(st, &rpr, &mut buf);
                        let mut d = Drawn::default();
                        self.draw(c, &mut d);
                        st.items.extend(d.inline.into_iter().map(Item::Node));
                        st.sides.extend(d.sides);
                    }
                }
                "w:ruby" => {
                    if let Some(base) = child(c, "w", "rubyBase") {
                        self.flush(st, &rpr, &mut buf);
                        for rr in children(base, "w", "r") {
                            self.run(rr, st);
                        }
                    }
                }
                _ => {}
            }
        }
        self.flush(st, &rpr, &mut buf);
    }

    // ---- notes ----------------------------------------------------------------------------

    fn note_number(&mut self, endnote: bool, id: &str) -> String {
        let pos = self.notes.iter().filter(|n| n.endnote == endnote).position(|n| n.id == id);
        let idx = match pos {
            Some(p) => p,
            None => {
                self.notes.push(NoteRef { endnote, id: id.to_string() });
                self.notes.iter().filter(|n| n.endnote == endnote).count() - 1
            }
        };
        if endnote {
            roman_lower(idx + 1)
        } else {
            (idx + 1).to_string()
        }
    }

    /// `div.notes` chunks with the referenced footnotes then endnotes.
    pub fn render_notes(&mut self) -> Vec<Node> {
        const CHUNK: usize = 1200;
        let mut out: Vec<Node> = Vec::new();
        let mut cur = Node::el("div").class("notes");
        let mut cur_w = 0usize;
        for endnote in [false, true] {
            let refs: Vec<String> = self.notes.iter().filter(|n| n.endnote == endnote).map(|n| n.id.clone()).collect();
            if refs.is_empty() {
                continue;
            }
            let suffix = if endnote { "/endnotes" } else { "/footnotes" };
            let Some(part) = self.rels_main_part(suffix) else { continue };
            let Some(text) = self.pkg.text(&part) else { continue };
            let Ok(doc) = roxmltree::Document::parse_with_options(&text, xml::OPTS) else { continue };
            let saved_rels = std::mem::replace(&mut self.rels, self.pkg.rels(&part).unwrap_or_default());
            let saved_outline = std::mem::replace(&mut self.outline_on, false);
            let saved_avail = self.avail;
            for (i, id) in refs.iter().enumerate() {
                let Some(note) = children(doc.root_element(), "w", if endnote { "endnote" } else { "footnote" })
                    .find(|n| attr(*n, "w", "id") == Some(id.as_str()))
                else {
                    continue;
                };
                let label = if endnote { roman_lower(i + 1) } else { (i + 1).to_string() };
                self.prefix = Some(("sup".into(), format!("{label} ")));
                let mut nodes = Vec::new();
                self.blocks(note, &mut nodes);
                self.prefix = None;
                for n in nodes {
                    cur_w += n.weight();
                    cur.push(n);
                }
                if cur_w >= CHUNK {
                    out.push(std::mem::replace(&mut cur, Node::el("div").class("notes")));
                    cur_w = 0;
                }
            }
            self.rels = saved_rels;
            self.outline_on = saved_outline;
            self.avail = saved_avail;
        }
        if !cur.children.is_empty() {
            out.push(cur);
        }
        out
    }

    fn rels_main_part(&mut self, suffix: &str) -> Option<String> {
        let main = self.pkg.main().to_string();
        let rels = self.pkg.rels(&main).ok()?;
        super::rel_part(&rels, suffix)
    }

    // ---- headers / footers ----------------------------------------------------------------

    /// The default header / footer of `sect` as blocks, or `None` when absent or blank.
    pub fn header_footer(&mut self, sect: X, kind: &str) -> Option<Vec<Node>> {
        let r = children(sect, "w", kind).find(|r| attr(*r, "w", "type") == Some("default"))?;
        let id = attr(r, "r", "id")?;
        let part = self.rels.get(id)?.target.clone();
        let text = self.pkg.text(&part)?;
        let doc = roxmltree::Document::parse_with_options(&text, xml::OPTS).ok()?;
        let saved_rels = std::mem::replace(&mut self.rels, self.pkg.rels(&part).unwrap_or_default());
        let saved_fields = std::mem::take(&mut self.fields);
        let mut nodes = Vec::new();
        self.blocks(doc.root_element(), &mut nodes);
        self.rels = saved_rels;
        self.fields = saved_fields;
        nodes.iter().any(super::has_content).then_some(nodes)
    }
}

/// Text of the first `limit` chars of a paragraph's `w:t`s (to tell its script).
fn paragraph_text(p: X, limit: usize) -> String {
    let mut s = String::new();
    for d in p.descendants() {
        if is(d, "w", "t") {
            s.push_str(&xml::text(d));
            if s.chars().count() >= limit {
                break;
            }
        }
    }
    s
}

/// Builds the `p`: the lead nodes, then the items; a leading plain run becomes the `p`'s own
/// text (it is drawn before the children).
fn build_p(cls: &str, pc: &PClass, lead: Vec<Node>, items: Vec<Item>) -> Node {
    let mut p = Node::el("p").class(cls);
    let mut kids: Vec<Node> = lead;
    for it in items {
        match it {
            Item::Seg(s) => {
                let mut c = String::new();
                if let Some(k) = &s.key {
                    key_diff(k, pc, &mut c);
                }
                c.push(' ');
                c.push_str(&s.extra);
                let mut n = Node::el("span").class(c.trim()).text(s.text);
                if let Some(h) = s.href {
                    n = n.prop("href", h);
                }
                kids.push(n);
            }
            Item::Node(n) => kids.push(n),
            Item::PageBreak => {}
        }
    }
    if kids.is_empty() {
        return p.text("\u{00A0}");
    }
    let plain_first = kids[0].kind == "el"
        && kids[0].props.get("class").is_none()
        && kids[0].props.get("href").is_none()
        && kids[0].children.is_empty()
        && kids[0].has_text();
    if plain_first {
        let first = kids.remove(0);
        if let Some(t) = first.props.get("text").and_then(|t| t.as_str()) {
            p = p.text(t);
        }
    }
    p.children = kids;
    p
}
