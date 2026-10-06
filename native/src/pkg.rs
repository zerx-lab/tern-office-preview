//! The OPC (zip) package under a .docx / .xlsx: parts by name, relationships, the main part.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// Parts larger than this are refused (a zip bomb, or nothing a preview can show).
const MAX_PART: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Docx,
    Xlsx,
}

/// One relationship of a part: `Id` → target part name (absolute, no leading `/`) or external URL.
#[derive(Clone, Debug)]
pub struct Rel {
    pub kind: String,
    pub target: String,
    pub external: bool,
}

pub type Rels = HashMap<String, Rel>;

pub struct Package {
    zip: zip::ZipArchive<BufReader<File>>,
    /// Lower-cased part name → name as stored, for case-insensitive lookup (OPC names are).
    names: HashMap<String, String>,
    main: String,
    kind: Kind,
}

const REL_OFFICE_DOC: &str = "/officeDocument";

impl Package {
    pub fn open(path: &Path) -> Result<Package, String> {
        let mut f = File::open(path).map_err(|e| format!("无法打开文件: {e}"))?;
        let mut magic = [0u8; 8];
        let n = f.read(&mut magic).unwrap_or(0);
        if n >= 8 && magic == [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1] {
            return Err(
                "这是旧版二进制格式（.doc / .xls）或加密的 Office 文件，无法预览；请另存为 .docx / .xlsx".into(),
            );
        }
        if n < 4 || &magic[..4] != b"PK\x03\x04" {
            return Err("不是 Office Open XML 文件（zip 包头缺失）".into());
        }
        let f = File::open(path).map_err(|e| format!("无法打开文件: {e}"))?;
        let zip = zip::ZipArchive::new(BufReader::new(f)).map_err(|e| format!("zip 包损坏: {e}"))?;
        let names = zip
            .file_names()
            .map(|n| (n.trim_start_matches('/').to_ascii_lowercase(), n.to_string()))
            .collect();
        let mut pkg = Package { zip, names, main: String::new(), kind: Kind::Docx };
        let root = pkg.rels("")?;
        let main = root
            .values()
            .find(|r| r.kind.ends_with(REL_OFFICE_DOC) && !r.external)
            .map(|r| r.target.clone())
            .or_else(|| {
                ["word/document.xml", "xl/workbook.xml"]
                    .iter()
                    .find(|p| pkg.has(p))
                    .map(|p| p.to_string())
            })
            .ok_or("找不到主文档部件（不是 Word / Excel 文件？）")?;
        let lower = main.to_ascii_lowercase();
        pkg.kind = if lower.ends_with("workbook.xml") || lower.starts_with("xl/") {
            Kind::Xlsx
        } else if lower.starts_with("ppt/") {
            return Err("这是 PowerPoint 文件，暂不支持预览".into());
        } else {
            Kind::Docx
        };
        pkg.main = main;
        Ok(pkg)
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Name of the main part (`word/document.xml`, `xl/workbook.xml`).
    pub fn main(&self) -> &str {
        &self.main
    }

    pub fn has(&self, part: &str) -> bool {
        self.names.contains_key(&part.trim_start_matches('/').to_ascii_lowercase())
    }

    /// The bytes of a part, or `None` when it doesn't exist.
    pub fn bytes(&mut self, part: &str) -> Option<Vec<u8>> {
        let stored = self.names.get(&part.trim_start_matches('/').to_ascii_lowercase())?.clone();
        let mut f = self.zip.by_name(&stored).ok()?;
        if f.size() > MAX_PART {
            return None;
        }
        let mut buf = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut buf).ok()?;
        Some(buf)
    }

    /// A part as text (UTF-8, or UTF-16 with a BOM), or `None`.
    pub fn text(&mut self, part: &str) -> Option<String> {
        let b = self.bytes(part)?;
        Some(decode_text(b))
    }

    /// Relationships of `part` (`""` for the package's own), targets resolved to part names.
    pub fn rels(&mut self, part: &str) -> Result<Rels, String> {
        let (dir, file) = match part.rfind('/') {
            Some(i) => (&part[..i], &part[i + 1..]),
            None => ("", part),
        };
        let rels_name =
            if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") };
        let Some(text) = self.text(&rels_name) else { return Ok(Rels::new()) };
        let doc = roxmltree::Document::parse_with_options(&text, crate::xml::OPTS)
            .map_err(|e| format!("{rels_name}: {e}"))?;
        let mut out = Rels::new();
        for r in doc.root_element().children().filter(|n| n.is_element()) {
            let (Some(id), Some(target)) = (r.attribute("Id"), r.attribute("Target")) else { continue };
            let external = r.attribute("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External"));
            let kind = r.attribute("Type").unwrap_or("").to_string();
            let target = if external { target.to_string() } else { resolve(dir, target) };
            out.insert(id.to_string(), Rel { kind, target, external });
        }
        Ok(out)
    }
}

/// Resolves a relationship target relative to the source part's directory.
pub fn resolve(dir: &str, target: &str) -> String {
    let target = target.split('#').next().unwrap_or("");
    let joined = if let Some(abs) = target.strip_prefix('/') {
        abs.to_string()
    } else if dir.is_empty() {
        target.to_string()
    } else {
        format!("{dir}/{target}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    percent_decode(&parts.join("/"))
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Decodes XML bytes: UTF-8 (BOM stripped) or UTF-16 LE/BE with a BOM.
pub fn decode_text(b: Vec<u8>) -> String {
    if b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&b[3..]).into_owned();
    }
    if b.len() >= 2 && (b[0] == 0xFF && b[1] == 0xFE || b[0] == 0xFE && b[1] == 0xFF) {
        let le = b[0] == 0xFF;
        let units: Vec<u16> = b[2..]
            .chunks_exact(2)
            .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
            .collect();
        let s = String::from_utf16_lossy(&units);
        // roxmltree refuses a declaration that names UTF-16 for a Rust string.
        return strip_decl(&s);
    }
    match String::from_utf8(b) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    }
}

fn strip_decl(s: &str) -> String {
    if s.starts_with("<?xml") {
        if let Some(end) = s.find("?>") {
            return s[end + 2..].to_string();
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_targets() {
        assert_eq!(resolve("word", "media/image1.png"), "word/media/image1.png");
        assert_eq!(resolve("xl/worksheets", "../drawings/drawing1.xml"), "xl/drawings/drawing1.xml");
        assert_eq!(resolve("xl", "/xl/sharedStrings.xml"), "xl/sharedStrings.xml");
        assert_eq!(resolve("", "word/document.xml"), "word/document.xml");
        assert_eq!(resolve("word", "media/a%20b.png"), "word/media/a b.png");
    }
}
