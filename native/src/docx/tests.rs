//! End-to-end tests: synthetic packages rendered into a temp cache, then the part JSON inspected.

use super::*;
use serde_json::Value;
use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

static N: AtomicUsize = AtomicUsize::new(0);

struct Rendered {
    meta: Value,
    parts: Vec<Value>,
}

fn png() -> Vec<u8> {
    let mut v = Cursor::new(Vec::new());
    image::RgbaImage::new(4, 2).write_to(&mut v, image::ImageFormat::Png).unwrap();
    v.into_inner()
}

/// Builds a .docx with `body` as `w:body` content and the given extra parts / relationships.
fn build(body: &str, extra: &[(&str, &[u8])], rels: &[(&str, &str, &str)]) -> PathBuf {
    let id = N.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("opv-docx-test-{}-{id}", std::process::id()));
    std::fs::create_dir_all(dir.join("out/img")).unwrap();
    let path = dir.join("t.docx");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let opts = zip::write::SimpleFileOptions::default();
    let mut put = |name: &str, data: &[u8]| {
        zip.start_file(name, opts).unwrap();
        zip.write_all(data).unwrap();
    };
    put(
        "_rels/.rels",
        format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#).as_bytes(),
    );
    let mut rel_xml = String::new();
    for (id, kind, target) in rels {
        let mode = if target.starts_with("http") { r#" TargetMode="External""# } else { "" };
        rel_xml.push_str(&format!(r#"<Relationship Id="{id}" Type="{REL}/{kind}" Target="{target}"{mode}/>"#));
    }
    put(
        "word/_rels/document.xml.rels",
        format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rel_xml}</Relationships>"#).as_bytes(),
    );
    put("word/document.xml", format!(r#"<w:document {W}><w:body>{body}</w:body></w:document>"#).as_bytes());
    for (name, data) in extra {
        put(name, data);
    }
    zip.finish().unwrap();
    path
}

fn render_path(path: &std::path::Path) -> Rendered {
    let dir = path.parent().unwrap().join("out");
    let mut pkg = Package::open(path).unwrap();
    let mut out = Out::new(dir.clone());
    let meta = render(&mut pkg, &mut out).unwrap();
    let parts = meta["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let f = dir.join(format!("{}.json", p["file"].as_str().unwrap()));
            serde_json::from_str::<Value>(&std::fs::read_to_string(f).unwrap()).unwrap()["node"].clone()
        })
        .collect();
    Rendered { meta, parts }
}

fn doc(body: &str) -> Rendered {
    render_path(&build(body, &[], &[]))
}

fn doc_with(body: &str, extra: &[(&str, &[u8])], rels: &[(&str, &str, &str)]) -> Rendered {
    render_path(&build(body, extra, rels))
}

fn walk<'a>(v: &'a Value, f: &mut dyn FnMut(&'a Value)) {
    f(v);
    if let Some(c) = v["c"].as_array() {
        for c in c {
            walk(c, f);
        }
    }
}

fn all(v: &Value, pred: impl Fn(&Value) -> bool) -> Vec<&Value> {
    let mut out = Vec::new();
    walk(v, &mut |n| {
        if pred(n) {
            out.push(n);
        }
    });
    out
}

fn tag<'a>(v: &'a Value, t: &'a str) -> Vec<&'a Value> {
    all(v, |n| n["p"]["tag"] == t)
}

fn class(n: &Value) -> Vec<&str> {
    n["p"]["class"].as_str().unwrap_or("").split_whitespace().collect()
}

/// All text of a node tree (own text before children).
fn text_of(v: &Value) -> String {
    let mut s = String::new();
    walk(v, &mut |n| {
        if n["k"] == "math" {
            return;
        }
        if let Some(t) = n["p"]["text"].as_str() {
            s.push_str(t);
        }
    });
    s
}

fn para(style: Option<&str>, inner: &str) -> String {
    let ppr = style.map(|s| format!(r#"<w:pPr><w:pStyle w:val="{s}"/></w:pPr>"#)).unwrap_or_default();
    format!("<w:p>{ppr}{inner}</w:p>")
}

fn run(t: &str) -> String {
    format!("<w:r><w:t xml:space=\"preserve\">{t}</w:t></w:r>")
}

const STYLES: &str = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="22"/></w:rPr></w:rPrDefault></w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
  <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/>
    <w:pPr><w:spacing w:before="240" w:after="120"/><w:outlineLvl w:val="0"/></w:pPr>
    <w:rPr><w:b/><w:color w:val="2F5496"/><w:sz w:val="32"/></w:rPr></w:style>
  <w:style w:type="character" w:styleId="Emph"><w:name w:val="Emph"/><w:rPr><w:i/><w:color w:val="FF0000"/></w:rPr></w:style>
</w:styles>"#;

fn styles_rel() -> [(&'static str, &'static str, &'static str); 1] {
    [("rId10", "styles", "styles.xml")]
}

#[test]
fn style_inheritance_reaches_classes_and_outline() {
    let body = format!(
        "{}{}{}",
        para(Some("Heading1"), &run("Title")),
        para(None, &format!("<w:r><w:rPr><w:rStyle w:val=\"Emph\"/></w:rPr><w:t>em</w:t></w:r>{}", run(" plain"))),
        "<w:p><w:pPr><w:jc w:val=\"center\"/><w:ind w:left=\"720\" w:right=\"360\"/><w:spacing w:before=\"300\" w:line=\"360\" w:lineRule=\"auto\"/></w:pPr><w:r><w:t>mid</w:t></w:r></w:p>"
    );
    let r = doc_with(&body, &[("word/styles.xml", STYLES.as_bytes())], &styles_rel());
    let ps = tag(&r.parts[0], "p");
    // Heading: bold, 16pt, theme-less color, margins from the style chain.
    let h = class(ps[0]);
    for c in ["b", "f32", "c124", "mt8", "mb4"] {
        assert!(h.contains(&c), "{c} in {h:?}");
    }
    assert_eq!(r.meta["outline"][0]["text"], "Title");
    assert_eq!(r.meta["outline"][0]["level"], 1);
    assert_eq!(r.meta["outline"][0]["part"], 0);
    // Body paragraph: default size, character style adds italic + red on the run only.
    assert!(class(ps[1]).contains(&"f22"));
    let em = tag(ps[1], "span").into_iter().find(|s| s["p"]["text"] == "em").expect("em span");
    let ec = class(em);
    assert!(ec.contains(&"i") && ec.contains(&"c700"), "{ec:?}");
    // Direct formatting: alignment, indents (twips/15 → 4px steps), spacing, line height.
    let d = class(ps[2]);
    for c in ["ac", "pl12", "pr6", "mt10"] {
        assert!(d.contains(&c), "{c} in {d:?}");
    }
    assert!(d.iter().any(|c| c.starts_with("lh3")), "{d:?}");
}

#[test]
fn indents_hanging_and_first_line_use_spacers() {
    let body = "<w:p><w:pPr><w:ind w:firstLine=\"480\"/></w:pPr><w:r><w:t>first</w:t></w:r></w:p>\
                <w:p><w:pPr><w:ind w:left=\"720\" w:hanging=\"480\"/></w:pPr><w:r><w:t>hang</w:t></w:r></w:p>";
    let r = doc(body);
    let ps = tag(&r.parts[0], "p");
    let sp = &ps[0]["c"][0];
    assert!(class(sp).contains(&"in") && class(sp).contains(&"w8"), "{:?}", class(sp));
    assert!(!class(ps[0]).iter().any(|c| c.starts_with("pl")));
    assert!(class(ps[1]).contains(&"pl12"));
    assert!(class(&ps[1]["c"][0]).contains(&"ml8"), "{:?}", ps[1]["c"][0]);
}

#[test]
fn numbered_paragraphs_get_labels() {
    let numbering = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
      <w:abstractNum w:abstractNumId="0">
        <w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl>
        <w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerRoman"/><w:lvlText w:val="%1.%2)"/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl>
      </w:abstractNum>
      <w:abstractNum w:abstractNumId="1">
        <w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val="&#xF0B7;"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr><w:rPr><w:rFonts w:ascii="Symbol"/></w:rPr></w:lvl>
      </w:abstractNum>
      <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
      <w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>
    </w:numbering>"#;
    let li = |id: u32, lvl: u8, t: &str| {
        format!("<w:p><w:pPr><w:numPr><w:ilvl w:val=\"{lvl}\"/><w:numId w:val=\"{id}\"/></w:numPr></w:pPr>{}</w:p>", run(t))
    };
    let body = format!("{}{}{}{}{}", li(1, 0, "a"), li(1, 1, "b"), li(1, 1, "c"), li(1, 0, "d"), li(2, 0, "e"));
    let r = doc_with(&body, &[("word/numbering.xml", numbering.as_bytes())], &[("rId11", "numbering", "numbering.xml")]);
    let labels: Vec<String> = all(&r.parts[0], |n| class(n).contains(&"lbl")).iter().map(|n| n["p"]["text"].as_str().unwrap().trim().to_string()).collect();
    assert_eq!(labels, ["1.", "1.i)", "1.ii)", "2.", "•"]);
    // Hanging label: pulled left by the hanging indent, paragraph padded by the left indent.
    let first = tag(&r.parts[0], "p")[0];
    assert!(class(first).contains(&"pl12"));
    let lbl = &first["c"][0];
    assert!(class(lbl).contains(&"ml6") && class(lbl).contains(&"w6"), "{:?}", class(lbl));
}

#[test]
fn table_spans_merges_and_style_conditions() {
    let styles = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
      <w:style w:type="table" w:styleId="TS"><w:name w:val="TS"/>
        <w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:color="FF0000"/><w:insideH w:val="single" w:sz="4"/></w:tblBorders></w:tblPr>
        <w:tblStylePr w:type="firstRow"><w:rPr><w:b/></w:rPr><w:tcPr><w:shd w:val="clear" w:fill="D9E2F3"/></w:tcPr></w:tblStylePr>
        <w:tblStylePr w:type="band2Horz"><w:tcPr><w:shd w:val="clear" w:fill="F2F2F2"/></w:tcPr></w:tblStylePr>
      </w:style></w:styles>"#;
    let cell = |w: u32, extra: &str, t: &str| {
        format!("<w:tc><w:tcPr><w:tcW w:w=\"{w}\" w:type=\"dxa\"/>{extra}</w:tcPr><w:p>{}</w:p></w:tc>", run(t))
    };
    let tbl = format!(
        "<w:tbl><w:tblPr><w:tblStyle w:val=\"TS\"/><w:tblLook w:val=\"0420\"/></w:tblPr>\
         <w:tblGrid><w:gridCol w:w=\"1500\"/><w:gridCol w:w=\"1500\"/><w:gridCol w:w=\"3000\"/></w:tblGrid>\
         <w:tr>{}{}</w:tr>\
         <w:tr>{}{}{}</w:tr>\
         <w:tr>{}{}{}</w:tr>\
         <w:tr>{}{}{}</w:tr></w:tbl>",
        cell(3000, "<w:gridSpan w:val=\"2\"/>", "head"),
        cell(3000, "", "h3"),
        cell(1500, "<w:vMerge w:val=\"restart\"/>", "m"),
        cell(1500, "", "x"),
        cell(3000, "", "y"),
        cell(1500, "<w:vMerge/>", ""),
        cell(1500, "", "p"),
        cell(3000, "", "q"),
        cell(1500, "<w:vMerge/>", ""),
        cell(1500, "", "r"),
        cell(3000, "", "s"),
    );
    let r = doc_with(&tbl, &[("word/styles.xml", styles.as_bytes())], &styles_rel());
    let grid = all(&r.parts[0], |n| class(n).contains(&"dg"))[0];
    assert!(class(grid).contains(&"gt3"), "{:?}", class(grid));
    assert!(tag(&r.parts[0], "table").is_empty() && tag(&r.parts[0], "td").is_empty());
    let cells = grid["c"].as_array().unwrap();
    // Row-major: head, h3 | m, x, y | p, q | r, s (vMerge continuations are omitted).
    let texts: Vec<String> = cells.iter().map(text_of).collect();
    assert_eq!(texts, ["head", "h3", "m", "x", "y", "p", "q", "r", "s"]);
    assert!(cells.iter().all(|c| c["p"]["attrs"].is_null()));
    let c = |i: usize| class(&cells[i]);
    // gridSpan → span class and the summed width of the spanned grid columns (200px → w50).
    assert!(c(0).contains(&"gs2") && c(0).contains(&"w50"), "{:?}", c(0));
    assert!(c(1).contains(&"w50") && !c(1).iter().any(|k| k.starts_with("gs")));
    // vMerge restart spans the three rows.
    assert!(c(2).contains(&"gr3") && c(2).contains(&"w25"), "{:?}", c(2));
    // firstRow condition: shading D9E2F3 and bold paragraph text inside the header cells.
    assert!(c(0).contains(&"k667"), "{:?}", c(0));
    assert!(tag(&cells[0], "p").iter().any(|p| class(p).contains(&"b")));
    // Borders: the style's red top border on row 0; the first column / row draw left / top only.
    assert!(c(0).contains(&"bt1") && c(0).iter().any(|k| k.starts_with("bct")), "{:?}", c(0));
    // Interior edges are drawn once: by the cell above (bottom), not by the cell below (top).
    assert!(c(0).contains(&"bb1"));
    assert!(!c(3).contains(&"bt1"), "{:?}", c(3));
    assert!(c(3).contains(&"bb1"));
    // band: with firstRow on, row 1 is band1, row 2 is band2 (shaded F2F2F2 → k777).
    assert!(c(5).contains(&"k777"), "{:?}", c(5));
    assert!(!c(3).contains(&"k777"));
}

#[test]
fn shared_edges_come_from_the_neighbour_and_gaps_get_placeholders() {
    let cell = |extra: &str, t: &str| format!("<w:tc><w:tcPr><w:tcW w:w=\"1500\" w:type=\"dxa\"/>{extra}</w:tcPr><w:p>{}</w:p></w:tc>", run(t));
    let left_only = "<w:tcBorders><w:left w:val=\"single\" w:sz=\"4\"/><w:top w:val=\"single\" w:sz=\"4\"/></w:tcBorders>";
    let tbl = format!(
        "<w:tbl><w:tblGrid><w:gridCol w:w=\"1500\"/><w:gridCol w:w=\"1500\"/><w:gridCol w:w=\"1500\"/></w:tblGrid>\
         <w:tr>{}{}{}</w:tr>\
         <w:tr><w:trPr><w:gridBefore w:val=\"1\"/></w:trPr>{}{}</w:tr></w:tbl>",
        cell("", "a"), cell(left_only, "b"), cell("", "c"), cell(left_only, "d"), cell("", "e"),
    );
    let r = doc(&tbl);
    let grid = all(&r.parts[0], |n| class(n).contains(&"dg"))[0];
    let cells = grid["c"].as_array().unwrap();
    let texts: Vec<String> = cells.iter().map(text_of).collect();
    // The gridBefore column is an empty placeholder cell with a width.
    assert_eq!(texts, ["a", "b", "c", "", "d", "e"]);
    assert!(class(&cells[3]).contains(&"w25"));
    // b's own left border becomes a's right border (a draws it); b does not draw it again.
    assert!(class(&cells[0]).contains(&"br1"), "{:?}", class(&cells[0]));
    assert!(!class(&cells[1]).contains(&"bl1"), "{:?}", class(&cells[1]));
    // b's top border is on the first row, so b draws it itself.
    assert!(class(&cells[1]).contains(&"bt1"));
    // d's left border: its left neighbour is the placeholder (draws nothing), so d keeps it.
    assert!(class(&cells[4]).contains(&"bl1"), "{:?}", class(&cells[4]));
    // d's top border faces b, which is above: b draws its bottom edge from d's top.
    assert!(class(&cells[1]).contains(&"bb1"), "{:?}", class(&cells[1]));
}

#[test]
fn field_results_are_shown_and_instructions_hidden() {
    let body = "<w:p>\
        <w:r><w:t>See </w:t></w:r>\
        <w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>\
        <w:r><w:instrText xml:space=\"preserve\"> HYPERLINK \"https://example.com/a\" </w:instrText></w:r>\
        <w:r><w:fldChar w:fldCharType=\"separate\"/></w:r>\
        <w:r><w:rPr><w:b/></w:rPr><w:t>the site</w:t></w:r>\
        <w:r><w:fldChar w:fldCharType=\"end\"/></w:r>\
        <w:r><w:t> page </w:t></w:r>\
        <w:fldSimple w:instr=\" PAGE \"><w:r><w:t>7</w:t></w:r></w:fldSimple>\
        <w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText> TOC \\o </w:instrText></w:r>\
        <w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>cached</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r>\
        <w:r><w:rPr><w:vanish/></w:rPr><w:t>hidden</w:t></w:r>\
        <w:del w:id=\"1\"><w:r><w:delText>deleted</w:delText></w:r></w:del>\
        <w:ins w:id=\"2\"><w:r><w:t>inserted</w:t></w:r></w:ins>\
        </w:p>";
    let r = doc(body);
    let t = text_of(&r.parts[0]);
    assert_eq!(t, "See the site page 7cachedinserted");
    assert!(!t.contains("HYPERLINK") && !t.contains("TOC") && !t.contains("deleted") && !t.contains("hidden"));
    let link = all(&r.parts[0], |n| n["p"]["href"] == "https://example.com/a");
    assert_eq!(link.len(), 1);
    assert_eq!(link[0]["p"]["text"], "the site");
}

#[test]
fn hyperlink_elements_carry_external_urls_only() {
    let body = "<w:p><w:hyperlink r:id=\"rId40\"><w:r><w:t>ext</w:t></w:r></w:hyperlink><w:hyperlink w:anchor=\"bm\"><w:r><w:t>inner</w:t></w:r></w:hyperlink></w:p>";
    let r = doc_with(body, &[], &[("rId40", "hyperlink", "https://example.com/z")]);
    let links = all(&r.parts[0], |n| n["p"]["href"].is_string());
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["p"]["href"], "https://example.com/z");
    assert!(class(links[0]).contains(&"lnk"));
    assert_eq!(text_of(&r.parts[0]), "extinner");
}

#[test]
fn math_runs_breaks_tabs_and_symbols() {
    let body = "<w:p><w:r><w:t>a</w:t><w:tab/><w:t>b</w:t><w:br/><w:t>c</w:t><w:noBreakHyphen/><w:sym w:font=\"Wingdings\" w:char=\"F0FC\"/></w:r>\
        <m:oMath><m:f><m:num><m:r><m:t>1</m:t></m:r></m:num><m:den><m:r><m:t>2</m:t></m:r></m:den></m:f></m:oMath></w:p>\
        <w:p><m:oMathPara><m:oMath><m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:oMath></m:oMathPara></w:p>";
    let r = doc(body);
    assert_eq!(text_of(tag(&r.parts[0], "p")[0]), "a\tb\nc\u{2011}✓");
    let maths = all(&r.parts[0], |n| n["k"] == "math");
    assert_eq!(maths.len(), 2);
    assert_eq!(maths[0]["p"]["text"], "\\frac{1}{2}");
    assert_eq!(maths[0]["p"]["display"], false);
    assert_eq!(maths[1]["p"]["text"], "{x}^{2}");
    assert_eq!(maths[1]["p"]["display"], true);
}

#[test]
fn page_breaks_split_paragraphs() {
    let body = "<w:p><w:r><w:t>before</w:t></w:r><w:r><w:br w:type=\"page\"/></w:r><w:r><w:t>after</w:t></w:r></w:p><w:p><w:r><w:br w:type=\"page\"/></w:r></w:p><w:p><w:r><w:t>end</w:t></w:r></w:p>";
    let r = doc(body);
    let pg = &r.parts[0]["c"][0];
    let kinds: Vec<String> = pg["c"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| if class(n).contains(&"pbr") { "pbr".to_string() } else { text_of(n) })
        .collect();
    assert_eq!(kinds, ["before", "pbr", "after", "pbr", "end"]);
}

#[test]
fn pictures_text_boxes_and_placeholders() {
    let png = png();
    let drawing = |inner: &str| {
        format!("<w:r><w:drawing><wp:inline><wp:extent cx=\"952500\" cy=\"476250\"/><wp:docPr id=\"1\" name=\"p\" descr=\"alt text\"/><a:graphic><a:graphicData>{inner}</a:graphicData></a:graphic></wp:inline></w:drawing></w:r>")
    };
    let pic = "<pic:pic><pic:blipFill><a:blip r:embed=\"rId20\"/></pic:blipFill></pic:pic>";
    let emf = "<pic:pic><pic:blipFill><a:blip r:embed=\"rId21\"/></pic:blipFill></pic:pic>";
    let chart = "<c:chart xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"/>";
    let txbx = "<wps:wsp xmlns:wps=\"http://schemas.microsoft.com/office/word/2010/wordprocessingShape\"><wps:txbx><w:txbxContent><w:p><w:r><w:t>in the box</w:t></w:r></w:p></w:txbxContent></wps:txbx></wps:wsp>";
    let alt = format!(
        "<w:r><mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice Requires=\"wps\"><w:drawing><wp:anchor><wp:extent cx=\"1905000\" cy=\"476250\"/><a:graphic><a:graphicData>{txbx}</a:graphicData></a:graphic></wp:anchor></w:drawing></mc:Choice><mc:Fallback><w:pict/></mc:Fallback></mc:AlternateContent></w:r>"
    );
    let body = format!("<w:p>{}{}{}{}{}</w:p>", drawing(pic), drawing(emf), drawing(chart), alt, run(" tail"));
    let r = doc_with(
        &body,
        &[("word/media/image1.png", &png), ("word/media/image2.emf", &[1, 0, 0, 0, 0, 0, 0, 0])],
        &[("rId20", "image", "media/image1.png"), ("rId21", "image", "media/image2.emf")],
    );
    let imgs = all(&r.parts[0], |n| n["k"] == "image");
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0]["p"]["w"], 100.0);
    assert_eq!(imgs[0]["p"]["h"], 50.0);
    assert_eq!(imgs[0]["p"]["alt"], "alt text");
    let phs = all(&r.parts[0], |n| class(n).contains(&"ph"));
    assert_eq!(phs.len(), 2);
    // The text box is a sibling after the paragraph, not inside it.
    let boxes = all(&r.parts[0], |n| class(n).contains(&"tbx"));
    assert_eq!(boxes.len(), 1);
    assert_eq!(text_of(boxes[0]), "in the box");
    let p = tag(&r.parts[0], "p")[0];
    assert!(!text_of(p).contains("in the box"));
    let notes: Vec<&str> = r.meta["notes"].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect();
    assert!(notes.iter().any(|n| n.contains("EMF")), "{notes:?}");
    assert!(notes.iter().any(|n| n.contains("图表")), "{notes:?}");
}

#[test]
fn header_footer_and_footnotes() {
    let hdr = r#"<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>HEADER</w:t></w:r></w:p></w:hdr>"#;
    let ftr = r#"<w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>FOOTER </w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p></w:ftr>"#;
    let notes = r#"<w:footnotes xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
        <w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>
        <w:footnote w:id="1"><w:p><w:r><w:footnoteRef/></w:r><w:r><w:t xml:space="preserve"> note one</w:t></w:r></w:p></w:footnote>
        <w:footnote w:id="2"><w:p><w:r><w:t>note two</w:t></w:r></w:p></w:footnote></w:footnotes>"#;
    let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>My Title</dc:title></cp:coreProperties>"#;
    let body = "<w:p><w:r><w:t>Body</w:t></w:r><w:r><w:footnoteReference w:id=\"2\"/></w:r><w:r><w:footnoteReference w:id=\"1\"/></w:r></w:p>\
        <w:sectPr><w:headerReference w:type=\"default\" r:id=\"rId30\"/><w:footerReference w:type=\"default\" r:id=\"rId31\"/><w:pgSz w:w=\"12240\" w:h=\"15840\"/><w:pgMar w:left=\"1800\" w:right=\"1800\" w:top=\"1440\" w:bottom=\"720\"/></w:sectPr>";
    let r = doc_with(
        body,
        &[("word/header1.xml", hdr.as_bytes()), ("word/footer1.xml", ftr.as_bytes()), ("word/footnotes.xml", notes.as_bytes()), ("docProps/core.xml", core.as_bytes())],
        &[("rId30", "header", "header1.xml"), ("rId31", "footer", "footer1.xml"), ("rId32", "footnotes", "footnotes.xml")],
    );
    assert_eq!(r.meta["title"], "My Title");
    let root = &r.parts[0];
    let pg = &root["c"][0];
    // Page: 816px wide (mw204), margins capped at 96px (pl24), bottom 48px (pb12).
    let c = class(pg);
    for k in ["pg", "mw204", "pl24", "pr24", "pt24", "pb12"] {
        assert!(c.contains(&k), "{k} in {c:?}");
    }
    let kids = pg["c"].as_array().unwrap();
    assert!(class(&kids[0]).contains(&"hf") && text_of(&kids[0]) == "HEADER");
    assert!(class(&kids[1]).contains(&"hfr"));
    let last = kids.last().unwrap();
    assert!(class(last).contains(&"hf") && text_of(last) == "FOOTER 1", "{}", text_of(last));
    let notes_div = all(root, |n| class(n).contains(&"notes"));
    assert_eq!(notes_div.len(), 1);
    // Numbered in order of reference: id 2 is footnote 1.
    let t = text_of(notes_div[0]);
    assert!(t.starts_with("1 note two") && t.contains("2  note one"), "{t:?}");
    // References in the body are superscripts.
    let body_p = tag(root, "p").into_iter().find(|p| text_of(p).starts_with("Body")).unwrap();
    let sup = all(body_p, |n| class(n).contains(&"sup"));
    assert_eq!(sup.iter().map(|n| n["p"]["text"].as_str().unwrap()).collect::<String>(), "12");
}

#[test]
fn sections_with_different_pages_start_new_pg() {
    let body = "<w:p><w:r><w:t>one</w:t></w:r></w:p>\
        <w:p><w:pPr><w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:left=\"1440\" w:right=\"1440\" w:top=\"1440\" w:bottom=\"1440\"/></w:sectPr></w:pPr><w:r><w:t>two</w:t></w:r></w:p>\
        <w:p><w:r><w:t>three</w:t></w:r></w:p>\
        <w:sectPr><w:pgSz w:w=\"16838\" w:h=\"11906\" w:orient=\"landscape\"/><w:pgMar w:left=\"1440\" w:right=\"1440\" w:top=\"1440\" w:bottom=\"1440\"/></w:sectPr>";
    let r = doc(body);
    let pgs = all(&r.parts[0], |n| class(n).contains(&"pg"));
    assert_eq!(pgs.len(), 2);
    assert!(class(pgs[0]).contains(&"mw198"), "{:?}", class(pgs[0]));
    assert!(class(pgs[1]).contains(&"mw281"), "{:?}", class(pgs[1]));
    assert_eq!(tag(pgs[0], "p").len(), 2);
}

fn piece(weight: usize, brk: bool, heading: u8) -> Piece {
    Piece { weight, brk, heading }
}

#[test]
fn split_prefers_headings_and_breaks_in_the_tail() {
    // Budget 100: a heading-1 at weight 80 (in the last 25%) is the cut.
    let mut p: Vec<Piece> = (0..8).map(|_| piece(10, false, 0)).collect();
    p.push(piece(10, false, 1)); // index 8, 80 before it
    p.push(piece(10, false, 0));
    p.push(piece(10, false, 0));
    p.push(piece(10, false, 0));
    let r = split(&p, 100);
    assert_eq!(r[0], (0, 8));
    assert_eq!(r[1].0, 8);
    assert_eq!(r.last().unwrap().1, p.len());
    // A heading too early (weight 30 < 75) is ignored: cut where the budget runs out.
    let mut p: Vec<Piece> = vec![piece(10, false, 0), piece(10, false, 0), piece(10, false, 0), piece(10, false, 2)];
    p.extend((0..10).map(|_| piece(10, false, 0)));
    assert_eq!(split(&p, 100)[0], (0, 10));
    // Cut after an explicit break in the tail.
    let mut p: Vec<Piece> = (0..8).map(|_| piece(10, false, 0)).collect();
    p.push(piece(5, true, 0));
    p.extend((0..6).map(|_| piece(10, false, 0)));
    let r = split(&p, 100);
    assert_eq!(r[0], (0, 9));
    // Oversized single block stays whole; every piece is covered exactly once.
    let p = vec![piece(10, false, 0), piece(500, false, 0), piece(10, false, 0)];
    let r = split(&p, 100);
    assert_eq!(r.first().unwrap().0, 0);
    assert_eq!(r.last().unwrap().1, 3);
    for w in r.windows(2) {
        assert_eq!(w[0].1, w[1].0);
    }
    assert!(split(&[], 100).is_empty());
}

#[test]
fn big_documents_are_split_into_parts_with_outline() {
    let mut body = String::new();
    for ch in 0..90 {
        body.push_str(&para(Some("Heading1"), &run(&format!("Chapter {ch}"))));
        for i in 0..60 {
            body.push_str(&para(None, &run(&format!("Paragraph {ch}.{i} with some words in it to make a line."))));
        }
    }
    let r = doc_with(&body, &[("word/styles.xml", STYLES.as_bytes())], &styles_rel());
    let parts = r.meta["parts"].as_array().unwrap();
    assert!(parts.len() >= 2, "{} parts", parts.len());
    assert_eq!(parts[0]["label"], "1");
    assert_eq!(parts[1]["label"], "2");
    for p in &r.parts {
        let mut n = 0;
        walk(p, &mut |_| n += 1);
        assert!(n <= 6000, "part of {n} nodes");
    }
    let outline = r.meta["outline"].as_array().unwrap();
    assert_eq!(outline.len(), 90);
    // Outline parts are non-decreasing and every part has a heading at its start (natural cut).
    let ps: Vec<u64> = outline.iter().map(|o| o["part"].as_u64().unwrap()).collect();
    assert!(ps.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(*ps.last().unwrap() as usize, parts.len() - 1);
    for (i, part) in r.parts.iter().enumerate().skip(1) {
        let first = tag(&part["c"][0], "p")[0];
        assert!(text_of(first).starts_with("Chapter"), "part {i} starts with {:?}", text_of(first));
    }
}

#[test]
fn huge_tables_are_cut_between_rows() {
    let mut rows = String::new();
    for i in 0..1500 {
        rows.push_str(&format!("<w:tr><w:tc><w:p>{}</w:p></w:tc><w:tc><w:p>{}</w:p></w:tc></w:tr>", run(&format!("a{i}")), run("b")));
    }
    let body = format!("<w:tbl><w:tblGrid><w:gridCol w:w=\"2000\"/><w:gridCol w:w=\"2000\"/></w:tblGrid>{rows}</w:tbl>");
    let r = doc(&body);
    assert!(r.parts.len() >= 2);
    let total: usize = r.parts.iter().map(|p| tag(p, "p").len()).sum();
    assert_eq!(total, 3000);
    for p in &r.parts {
        let mut n = 0;
        walk(p, &mut |_| n += 1);
        assert!(n <= 6000, "{n}");
    }
}

#[test]
fn empty_document_and_empty_paragraph() {
    let r = doc("");
    assert_eq!(r.parts.len(), 1);
    let r = doc("<w:p/>");
    assert_eq!(text_of(&r.parts[0]), "\u{00A0}");
}
