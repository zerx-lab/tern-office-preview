//! .xlsx rendering: workbook → sheets → HTML-table parts.
//!
//! * `numfmt` — Excel number formats (the heart of showing values the way Excel does)
//! * `styles` — `styles.xml` boiled down to one class string per cell format
//! * `strings` — shared / inline strings with rich-text runs
//! * `sheet` — the streamed worksheet model
//! * `table` — layout, merges, borders, overflow, part splitting and the JSON writer
//! * `drawing` — pictures and charts anchored in a sheet

mod drawing;
pub mod numfmt;
mod sheet;
mod strings;
mod styles;
mod table;

use crate::color::Theme;
use crate::node::Node;
use crate::out::Out;
use crate::pkg::{Package, Rel, Rels};
use crate::xml::{self, attr, child, elems, is};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use strings::Strings;
use styles::{ColorCtx, Styles};
use table::{Ctx, Plan};

/// What one rendered sheet adds to the meta.
struct SheetOut {
    rows: u32,
    cols: u32,
    parts: Vec<(String, String)>,
    notes: Vec<String>,
}

struct Book<'a> {
    styles: &'a Styles,
    strings: &'a Strings,

    date1904: bool,
}

fn rel_of<'a>(rels: &'a Rels, suffix: &str) -> Option<&'a Rel> {
    rels.values().find(|r| !r.external && r.kind.ends_with(suffix))
}

pub fn render(pkg: &mut Package, out: &mut Out) -> Result<Value, String> {
    let wb_part = pkg.main().to_string();
    let text = pkg.text(&wb_part).ok_or("无法读取 workbook.xml")?;
    let doc =
        roxmltree::Document::parse_with_options(&text, xml::OPTS).map_err(|e| format!("workbook.xml 解析失败: {e}"))?;
    let rels = pkg.rels(&wb_part)?;
    let root = doc.root_element();
    let date1904 =
        child(root, "x", "workbookPr").and_then(|n| n.attribute("date1904")).is_some_and(|v| v == "1" || v == "true");
    let active: usize = child(root, "x", "bookViews")
        .and_then(|b| child(b, "x", "workbookView"))
        .and_then(|v| xml::num(v.attribute("activeTab")))
        .unwrap_or(0);

    let theme = rel_of(&rels, "/theme")
        .and_then(|r| pkg.text(&r.target))
        .or_else(|| pkg.text("xl/theme/theme1.xml"))
        .map(|t| Theme::parse(&t))
        .unwrap_or_default();
    let styles_part = rel_of(&rels, "/styles").map(|r| r.target.clone()).unwrap_or_else(|| "xl/styles.xml".into());
    let styles = Styles::parse(pkg.text(&styles_part).as_deref(), &theme);
    let cctx = ColorCtx { theme: &theme, indexed: &styles.indexed };
    let sst_part =
        rel_of(&rels, "/sharedStrings").map(|r| r.target.clone()).unwrap_or_else(|| "xl/sharedStrings.xml".into());
    let strings = match pkg.bytes(&sst_part) {
        Some(b) => Strings::parse(&b, &cctx),
        None => Strings::default(),
    };
    let book = Book { styles: &styles, strings: &strings, date1904 };

    let sheets_el = child(root, "x", "sheets").ok_or("工作簿中没有工作表")?;
    let mut sheets = Vec::new();
    for (si, n) in elems(sheets_el).filter(|n| is(*n, "x", "sheet")).enumerate() {
        let name = n.attribute("name").unwrap_or("Sheet").to_string();
        let state = match n.attribute("state") {
            Some("hidden") => "hidden",
            Some("veryHidden") => "veryHidden",
            _ => "visible",
        };
        let rel = attr(n, "r", "id").and_then(|id| rels.get(id));
        let res = match rel {
            None => message_sheet(pkg, out, si, "找不到此工作表的内容", None),
            Some(rel) if rel.kind.ends_with("/worksheet") => match worksheet(pkg, out, si, &rel.target, &book, &cctx) {
                Ok(s) => Ok(s),
                Err(e) => message_sheet(pkg, out, si, &format!("工作表解析失败：{e}"), None),
            },
            Some(rel) if rel.kind.ends_with("/chartsheet") => {
                message_sheet(pkg, out, si, "这是图表工作表，不支持预览", Some(&rel.target))
            }
            Some(_) => message_sheet(pkg, out, si, "此类工作表（对话框 / 宏表）不支持预览", None),
        }?;
        sheets.push(json!({
            "name": name,
            "state": state,
            "rows": res.rows,
            "cols": res.cols,
            "parts": res.parts.iter().map(|(f, l)| json!({"file": f, "label": l})).collect::<Vec<_>>(),
            "notes": res.notes,
        }));
    }
    if sheets.is_empty() {
        return Err("工作簿中没有工作表".into());
    }
    let visible = |i: usize| sheets.get(i).is_some_and(|s| s["state"] == "visible");
    let active = if visible(active) { active } else { (0..sheets.len()).find(|i| visible(*i)).unwrap_or(0) };
    Ok(json!({ "kind": "xlsx", "active": active, "sheets": sheets, "notes": Vec::<String>::new() }))
}

/// A sheet shown as a single message (chart sheets, unreadable sheets), plus its drawing if any.
fn message_sheet(
    pkg: &mut Package,
    out: &mut Out,
    si: usize,
    msg: &str,
    part: Option<&str>,
) -> Result<SheetOut, String> {
    let mut nodes = Vec::new();
    let mut files = Vec::new();
    if let Some(part) = part {
        if let Some(text) = pkg.text(part) {
            let rels = pkg.rels(part).unwrap_or_default();
            let rid = roxmltree::Document::parse_with_options(&text, xml::OPTS).ok().and_then(|d| {
                d.descendants().find(|n| is(*n, "x", "drawing")).and_then(|n| attr(n, "r", "id")).map(String::from)
            });
            if let Some(rel) = rid.and_then(|id| rels.get(&id).cloned()) {
                let dr = drawing::parse(pkg, &rel.target, &|_| 64.0, &|_| 20.0);
                (nodes, files) = drawing::nodes(&dr, pkg, out);
            }
        }
    }

    let mut root = Node::el("div").class("opv opx").child(Node::el("div").class("xmsg").text(msg));
    for n in nodes {
        root.push(n);
    }
    let file = format!("s{si}-0");
    out.write_part_raw(&file, &root.into_value().to_string(), &files)?;
    Ok(SheetOut { rows: 0, cols: 0, parts: vec![(file, "1".into())], notes: vec![msg.to_string()] })
}

fn worksheet(
    pkg: &mut Package,
    out: &mut Out,
    si: usize,
    part: &str,
    book: &Book,
    cctx: &ColorCtx,
) -> Result<SheetOut, String> {
    let bytes = pkg.bytes(part).ok_or("缺少工作表部件")?;
    let srels = pkg.rels(part).unwrap_or_default();
    let sh = sheet::parse(&bytes, cctx, &srels, book.date1904);
    drop(bytes);
    let ctx = Ctx::new(&sh, book.styles, book.strings, book.date1904);
    let plan = Plan::build(&ctx);

    // Pictures and charts go after the table, in the last part.
    let mut extra_nodes = Vec::new();
    let mut files = Vec::new();
    let mut notes = plan.notes.clone();
    if let Some(rel) = sh.drawing_rid.as_ref().and_then(|id| srels.get(id)).filter(|r| !r.external) {
        let col_px = |c: u32| plan.all_px.get(c as usize).copied().unwrap_or(plan.default_px);
        let default_row = sh.default_row_ht.map_or(20.0, |h| h * 4.0 / 3.0);
        let row_px = |r: u32| match sh.rows.binary_search_by_key(&(r + 1), |x| x.r) {
            Ok(i) if sh.rows[i].ht > 0.0 => sh.rows[i].ht as f64 * 4.0 / 3.0,
            _ => default_row,
        };
        let dr = drawing::parse(pkg, &rel.target, &col_px, &row_px);
        (extra_nodes, files) = drawing::nodes(&dr, pkg, out);
        if dr.shapes > 0 {
            notes.push(format!("含 {} 个形状或文本框（未显示）", dr.shapes));
        }
    }
    if sh.has_cf {
        notes.push("含条件格式（未显示）".into());
    }
    let extra = extra_nodes.into_iter().map(|n| n.into_value().to_string()).collect::<Vec<_>>().join(",");

    let mut part_notes = notes.clone();
    if plan.parts.is_empty() && extra.is_empty() {
        part_notes.push("此工作表没有可显示的数据".into());
    }
    let nparts = plan.parts.len().max(1);
    write_parts(&ctx, &plan, &part_notes, &extra, &files, out, si, sh.no_grid, nparts)?;
    let parts = (0..nparts)
        .map(|i| (format!("s{si}-{i}"), if plan.parts.is_empty() { "1".to_string() } else { plan.label(i) }))
        .collect();
    Ok(SheetOut { rows: plan.used_rows, cols: plan.used_cols, parts, notes })
}

#[allow(clippy::too_many_arguments)]
fn write_parts(
    ctx: &Ctx,
    plan: &Plan,
    notes: &[String],
    extra: &str,
    files: &[String],
    out: &Out,
    si: usize,
    no_grid: bool,
    nparts: usize,
) -> Result<(), String> {
    let next = AtomicUsize::new(0);
    let errors: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let work = || loop {
        let i = next.fetch_add(1, Ordering::Relaxed);
        if i >= nparts {
            break;
        }
        let last = i + 1 == nparts;
        let json = plan.render_part(ctx, i, notes, if last { extra } else { "" }, no_grid);
        let imgs: &[String] = if last { files } else { &[] };
        if let Err(e) = out.write_part_raw(&format!("s{si}-{i}"), &json, imgs) {
            if let Ok(mut v) = errors.lock() {
                v.push(e);
            }
        }
    };
    let workers =
        if nparts < 4 { 1 } else { std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).min(8) };
    if workers == 1 {
        work();
    } else {
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(&work);
            }
        });
    }
    match errors.into_inner().ok().and_then(|v| v.into_iter().next()) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("opv-xlsx-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("img")).unwrap();
        d
    }

    fn write_zip(path: &std::path::Path, files: &[(&str, Vec<u8>)]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (n, b) in files {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    }

    fn png() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(8, 6, image::Rgba([200, 30, 30, 255]));
        let mut c = Cursor::new(Vec::new());
        img.write_to(&mut c, image::ImageFormat::Png).unwrap();
        c.into_inner()
    }

    const NS: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

    fn workbook_files() -> Vec<(&'static str, Vec<u8>)> {
        let s = |x: String| x.into_bytes();
        let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
        vec![
            ("[Content_Types].xml", s("<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"/>".into())),
            (
                "_rels/.rels",
                s(format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{rel}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#)),
            ),
            (
                "xl/workbook.xml",
                s(format!(
                    r#"<workbook {NS}><workbookPr date1904="1"/><bookViews><workbookView activeTab="1"/></bookViews><sheets>
<sheet name="数据" sheetId="1" r:id="rId1"/><sheet name="隐藏" sheetId="2" state="hidden" r:id="rId2"/><sheet name="图表页" sheetId="3" r:id="rId3"/><sheet name="空" sheetId="4" r:id="rId4"/></sheets></workbook>"#
                )),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                s(format!(
                    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="{rel}/worksheet" Target="worksheets/sheet1.xml"/>
<Relationship Id="rId2" Type="{rel}/worksheet" Target="worksheets/sheet2.xml"/>
<Relationship Id="rId3" Type="{rel}/chartsheet" Target="chartsheets/sheet1.xml"/>
<Relationship Id="rId4" Type="{rel}/worksheet" Target="worksheets/sheet3.xml"/>
<Relationship Id="rId5" Type="{rel}/styles" Target="styles.xml"/>
<Relationship Id="rId6" Type="{rel}/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#
                )),
            ),
            (
                "xl/styles.xml",
                s(format!(
                    r#"<styleSheet {NS}><fonts><font><sz val="11"/><name val="Calibri"/></font></fonts>
<fills><fill><patternFill patternType="none"/></fill></fills><borders><border/></borders>
<cellXfs><xf numFmtId="0"/><xf numFmtId="14"/><xf numFmtId="4"/></cellXfs></styleSheet>"#
                )),
            ),
            ("xl/sharedStrings.xml", s(format!(r#"<sst {NS}><si><t>名称</t></si><si><t>https://x.y</t></si></sst>"#))),
            (
                "xl/worksheets/sheet1.xml",
                s(format!(
                    r#"<worksheet {NS}><sheetData>
<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" s="1"><v>0</v></c><c r="C1" s="2"><v>1234.5</v></c></row>
<row r="2"><c r="A2"><f>SUM(C1:C1)</f><v></v></c></row></sheetData>
<conditionalFormatting sqref="A1"><cfRule type="expression" priority="1"/></conditionalFormatting>
<drawing r:id="rId1"/></worksheet>"#
                )),
            ),
            (
                "xl/worksheets/_rels/sheet1.xml.rels",
                s(format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{rel}/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#)),
            ),
            (
                "xl/drawings/drawing1.xml",
                s(r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
<xdr:oneCellAnchor><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>3</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx="1905000" cy="952500"/>
<xdr:pic><xdr:nvPicPr><xdr:cNvPr id="2" name="Logo" descr="标志"/><xdr:cNvPicPr/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed="rId1"/></xdr:blipFill><xdr:spPr/></xdr:pic><xdr:clientData/></xdr:oneCellAnchor>
<xdr:twoCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>9</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>4</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>20</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
<xdr:graphicFrame><xdr:nvGraphicFramePr><xdr:cNvPr id="3" name="销售图"/></xdr:nvGraphicFramePr><a:graphic><a:graphicData><c:chart r:id="rId2"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>
<xdr:twoCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>0</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
<xdr:sp><xdr:nvSpPr><xdr:cNvPr id="4" name="Box"/></xdr:nvSpPr></xdr:sp><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>"#.to_string()),
            ),
            (
                "xl/drawings/_rels/drawing1.xml.rels",
                s(format!(
                    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{rel}/image" Target="../media/image1.png"/><Relationship Id="rId2" Type="{rel}/chart" Target="../charts/chart1.xml"/></Relationships>"#
                )),
            ),
            ("xl/media/image1.png", png()),
            ("xl/worksheets/sheet2.xml", s(format!(r#"<worksheet {NS}><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#))),
            ("xl/worksheets/sheet3.xml", s(format!(r#"<worksheet {NS}><sheetData/></worksheet>"#))),
            ("xl/chartsheets/sheet1.xml", s(format!(r#"<chartsheet {NS}><drawing r:id="rId1"/></chartsheet>"#))),
        ]
    }

    #[test]
    fn renders_a_whole_workbook() {
        let dir = tmp("wb");
        let file = dir.join("t.xlsx");
        write_zip(&file, &workbook_files());
        let mut pkg = Package::open(&file).unwrap();
        let mut out = Out::new(dir.clone());
        let meta = render(&mut pkg, &mut out).unwrap();
        assert_eq!(meta["kind"], "xlsx");
        let sheets = meta["sheets"].as_array().unwrap();
        assert_eq!(sheets.len(), 4);
        // activeTab=1 is hidden → first visible
        assert_eq!(meta["active"], 0);
        assert_eq!(sheets[0]["name"], "数据");
        assert_eq!(sheets[1]["state"], "hidden");
        assert_eq!(sheets[0]["rows"], 2);
        assert_eq!(sheets[0]["cols"], 3);
        let notes: Vec<&str> = sheets[0]["notes"].as_array().unwrap().iter().filter_map(|n| n.as_str()).collect();
        assert!(notes.iter().any(|n| n.contains("条件格式")), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("1 个形状")), "{notes:?}");
        assert_eq!(sheets[0]["parts"][0]["file"], "s0-0");
        assert_eq!(sheets[0]["parts"][0]["label"], "1–2");

        let part: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("s0-0.json")).unwrap()).unwrap();
        let s = part["node"].to_string();
        // date1904: serial 0 in style 14 is 1904/1/1
        assert!(s.contains("1904/1/1"), "{s}");
        assert!(s.contains("1,234.50"), "{s}");
        assert!(s.contains("=SUM(C1:C1)"), "{s}");
        assert!(s.contains("图片 · 锚定于 B4"), "{s}");
        assert!(s.contains("图表：销售图 (不支持预览)"), "{s}");
        assert!(s.contains(r#""k":"image""#), "{s}");
        assert_eq!(part["images"].as_array().unwrap().len(), 1);
        assert_eq!(part["images"][0]["mime"], "image/png");
        assert!(dir.join("img").join("1.png").exists());

        // chartsheet: one part with a message and the chart placeholder
        let c: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("s2-0.json")).unwrap()).unwrap();
        assert!(c["node"].to_string().contains("图表工作表"));
        assert_eq!(sheets[2]["parts"].as_array().unwrap().len(), 1);
        // empty sheet: one part with a message
        let e: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("s3-0.json")).unwrap()).unwrap();
        assert!(e["node"].to_string().contains("没有可显示的数据"));
        assert_eq!(sheets[3]["parts"][0]["label"], "1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_sheet_does_not_fail_the_book() {
        let dir = tmp("broken");
        let file = dir.join("t.xlsx");
        let mut files = workbook_files();
        files.retain(|(n, _)| *n != "xl/worksheets/sheet2.xml");
        write_zip(&file, &files);
        let mut pkg = Package::open(&file).unwrap();
        let mut out = Out::new(dir.clone());
        let meta = render(&mut pkg, &mut out).unwrap();
        let notes = meta["sheets"][1]["notes"][0].as_str().unwrap();
        assert!(notes.contains("解析失败"), "{notes}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
