//! Pictures, text boxes, VML and unsupported graphics (charts, SmartArt, EMF) inside runs.

use super::Ctx;
use crate::node::{self, Node};
use crate::xml::{attr, child, elems, is, num, qname};
use roxmltree::Node as X;

/// What one drawing produced: nodes that sit in the line, and blocks that go after the paragraph.
#[derive(Default)]
pub struct Drawn {
    pub inline: Vec<Node>,
    pub sides: Vec<Node>,
}

impl Drawn {
    pub fn is_empty(&self) -> bool {
        self.inline.is_empty() && self.sides.is_empty()
    }

    fn merge(&mut self, o: Drawn) {
        self.inline.extend(o.inline);
        self.sides.extend(o.sides);
    }
}

/// `width:100pt` style measures of a VML `style` attribute, in px.
fn css_len(style: &str, key: &str) -> Option<f64> {
    let v = style.split(';').find_map(|kv| {
        let (k, v) = kv.split_once(':')?;
        (k.trim().eq_ignore_ascii_case(key)).then(|| v.trim())
    })?;
    let split = v.find(|c: char| c.is_ascii_alphabetic() || c == '%').unwrap_or(v.len());
    let n: f64 = v[..split].trim().parse().ok()?;
    Some(match &v[split..] {
        "pt" => n * 4.0 / 3.0,
        "in" => n * 96.0,
        "cm" => n * 96.0 / 2.54,
        "mm" => n * 96.0 / 25.4,
        "pc" => n * 16.0,
        "px" | "" => n,
        _ => return None,
    })
}

fn extent(n: X, pfx: &str, name: &str) -> (f64, f64) {
    child(n, pfx, name)
        .map(|e| {
            (
                num::<f64>(attr(e, "", "cx")).unwrap_or(0.0) / 9525.0,
                num::<f64>(attr(e, "", "cy")).unwrap_or(0.0) / 9525.0,
            )
        })
        .unwrap_or((0.0, 0.0))
}

impl Ctx<'_> {
    /// Draws a `w:drawing`, `w:pict`, `w:object` or `mc:AlternateContent` into `d`.
    pub fn draw(&mut self, n: X, d: &mut Drawn) {
        match qname(n).as_str() {
            "w:drawing" => {
                for e in elems(n) {
                    if is(e, "wp", "inline") || is(e, "wp", "anchor") {
                        self.draw_wp(e, d);
                    }
                }
            }
            "mc:AlternateContent" => {
                let mut tmp = Drawn::default();
                for ch in elems(n).filter(|c| is(*c, "mc", "Choice")) {
                    for e in elems(ch) {
                        self.draw(e, &mut tmp);
                    }
                    if !tmp.is_empty() {
                        break;
                    }
                }
                if tmp.is_empty() {
                    if let Some(fb) = elems(n).find(|c| is(*c, "mc", "Fallback")) {
                        for e in elems(fb) {
                            self.draw(e, &mut tmp);
                        }
                    }
                }
                d.merge(tmp);
            }
            "w:pict" | "w:object" | "w:control" => {
                for e in elems(n) {
                    self.vml(e, d);
                }
            }
            _ => {}
        }
    }

    fn draw_wp(&mut self, e: X, d: &mut Drawn) {
        let (w, h) = extent(e, "wp", "extent");
        let alt = child(e, "wp", "docPr")
            .and_then(|p| attr(p, "", "descr").filter(|s| !s.trim().is_empty()).or_else(|| attr(p, "", "title")))
            .unwrap_or("")
            .to_string();
        let Some(gd) = child(e, "a", "graphic").and_then(|g| child(g, "a", "graphicData")) else { return };
        self.graphic(gd, w, h, &alt, 1.0, d);
    }

    fn graphic(&mut self, gd: X, w: f64, h: f64, alt: &str, scale: f64, d: &mut Drawn) {
        for g in elems(gd) {
            match qname(g).as_str() {
                "pic:pic" => self.pic(g, w, h, alt, d),
                "wps:wsp" => self.wsp(g, w, h, d),
                "wpg:wgp" => self.group(g, scale, d),
                "c:chart" => {
                    self.count("chart");
                    d.inline.push(node::placeholder("[图表：暂不支持预览]"));
                }
                "mc:AlternateContent" => self.draw(g, d),
                _ => match g.tag_name().name() {
                    "relIds" => {
                        self.count("smartart");
                        d.inline.push(node::placeholder("[SmartArt 图形：暂不支持预览]"));
                    }
                    "wpc" => self.graphic(g, w, h, alt, scale, d),
                    _ => {}
                },
            }
        }
    }

    fn group(&mut self, g: X, scale: f64, d: &mut Drawn) {
        let mut sc = scale;
        if let Some(x) = child(g, "wpg", "grpSpPr").and_then(|p| child(p, "a", "xfrm")) {
            let ext = child(x, "a", "ext");
            let ch = child(x, "a", "chExt");
            if let (Some(e), Some(c)) = (ext, ch) {
                let (ew, cw) = (num::<f64>(attr(e, "", "cx")).unwrap_or(0.0), num::<f64>(attr(c, "", "cx")).unwrap_or(0.0));
                if ew > 0.0 && cw > 0.0 {
                    sc *= ew / cw;
                }
            }
        }
        for c in elems(g) {
            match qname(c).as_str() {
                "wps:wsp" => {
                    let (w, h) = child(c, "wps", "spPr").map(|p| spr_ext(p, sc)).unwrap_or((0.0, 0.0));
                    self.wsp(c, w, h, d);
                }
                "pic:pic" => {
                    let (w, h) = child(c, "pic", "spPr").map(|p| spr_ext(p, sc)).unwrap_or((0.0, 0.0));
                    self.pic(c, w, h, "", d);
                }
                "wpg:grpSp" | "wpg:wgp" => self.group(c, sc, d),
                "mc:AlternateContent" => self.draw(c, d),
                _ => {}
            }
        }
    }

    fn wsp(&mut self, g: X, w: f64, _h: f64, d: &mut Drawn) {
        if let Some(tb) = child(g, "wps", "txbx").and_then(|t| child(t, "w", "txbxContent")) {
            if let Some(n) = self.textbox(tb, w) {
                d.sides.push(n);
            }
            return;
        }
        // A picture-filled or plain shape without text.
        self.count("shape");
    }

    fn pic(&mut self, p: X, w: f64, h: f64, alt: &str, d: &mut Drawn) {
        let blip = p.descendants().find(|n| is(*n, "a", "blip"));
        let (w, h) = if w > 0.0 && h > 0.0 {
            (w, h)
        } else {
            child(p, "pic", "spPr").map(|s| spr_ext(s, 1.0)).unwrap_or((w, h))
        };
        match blip.and_then(|b| attr(b, "r", "embed")) {
            Some(id) => self.image_rel(id, w, h, alt, d),
            None => {
                if blip.and_then(|b| attr(b, "r", "link")).is_some() {
                    d.inline.push(node::placeholder("[链接的外部图片：无法预览]"));
                    self.count("img");
                }
            }
        }
    }

    /// An image by relationship id; failures become placeholders and are counted.
    fn image_rel(&mut self, id: &str, w: f64, h: f64, alt: &str, d: &mut Drawn) {
        let Some(rel) = self.rels.get(id).filter(|r| !r.external).map(|r| r.target.clone()) else {
            self.count("img");
            d.inline.push(node::placeholder("[图片缺失]"));
            return;
        };
        match self.out.media(self.pkg, &rel) {
            Ok(m) => {
                let (w, h) = match (w > 0.0, h > 0.0) {
                    (true, true) => (w, h),
                    (true, false) if m.w > 0 => (w, w * m.h as f64 / m.w as f64),
                    _ => (m.w as f64, m.h as f64),
                };
                d.inline.push(node::image(&m.file, w, h, alt));
            }
            Err(why) => {
                if why.starts_with("EMF") {
                    self.count("emf");
                    d.inline.push(node::placeholder("[EMF/WMF 图片：无法预览]"));
                } else {
                    self.count("img");
                    d.inline.push(node::placeholder(&format!("[图片无法显示：{why}]")));
                }
            }
        }
    }

    /// A text box: `div.tbx` holding the box's blocks, or `None` when it is empty.
    pub fn textbox(&mut self, tb: X, w: f64) -> Option<Node> {
        if self.depth >= 3 {
            return None;
        }
        let saved_fields = std::mem::take(&mut self.fields);
        let saved_tbl = self.tbl.take();
        let saved_avail = self.avail;
        let saved_outline = std::mem::replace(&mut self.outline_on, false);
        let saved_prefix = self.prefix.take();
        let mut cls = String::from("tbx");
        if w >= 40.0 && w <= self.avail {
            if let Some(c) = crate::css::width(w) {
                cls.push(' ');
                cls.push_str(&c);
            }
            self.avail = w - 20.0;
        } else {
            self.avail = (self.avail - 20.0).max(60.0);
        }
        self.depth += 1;
        let mut kids = Vec::new();
        self.blocks(tb, &mut kids);
        self.depth -= 1;
        self.fields = saved_fields;
        self.tbl = saved_tbl;
        self.avail = saved_avail;
        self.outline_on = saved_outline;
        self.prefix = saved_prefix;
        if !kids.iter().any(super::has_content) {
            return None;
        }
        let mut n = Node::el("div").class(&cls);
        n.children = kids;
        Some(n)
    }

    fn vml(&mut self, e: X, d: &mut Drawn) {
        let name = qname(e);
        match name.as_str() {
            "v:group" => {
                for c in elems(e) {
                    self.vml(c, d);
                }
            }
            "v:shapetype" | "o:OLEObject" | "o:lock" | "w10:wrap" | "v:stroke" | "v:fill" | "v:path" => {}
            n if n.starts_with("v:") => {
                let style = attr(e, "", "style").unwrap_or("");
                let w = css_len(style, "width").unwrap_or(0.0);
                let h = css_len(style, "height").unwrap_or(0.0);
                let alt = attr(e, "", "alt").unwrap_or("");
                if let Some(im) = elems(e).find(|c| is(*c, "v", "imagedata")) {
                    let id = attr(im, "r", "id").or_else(|| attr(im, "o", "relid"));
                    match id {
                        Some(id) => self.image_rel(id, w, h, alt, d),
                        None => self.count("img"),
                    }
                } else if let Some(tb) = elems(e).find(|c| is(*c, "v", "textbox")).and_then(|t| child(t, "w", "txbxContent")) {
                    if let Some(n) = self.textbox(tb, w) {
                        d.sides.push(n);
                    }
                } else if !matches!(n, "v:line" | "v:polyline" | "v:curve" | "v:arc") {
                    self.count("shape");
                }
            }
            _ => {}
        }
    }
}

/// `a:xfrm/a:ext` of a shape's `spPr`, scaled.
fn spr_ext(sppr: X, scale: f64) -> (f64, f64) {
    let (w, h) = extent(child(sppr, "a", "xfrm").unwrap_or(sppr), "a", "ext");
    (w * scale, h * scale)
}
