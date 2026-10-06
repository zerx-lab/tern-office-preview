//! `office-preview`: turns a .docx / .xlsx into Tern view trees on disk.
//!
//! ```text
//! office-preview render <file> --cache <dir>   render (or reuse) and print {"ok":true,"dir":…,"meta":…}
//! office-preview css                           print the style sheet the plugin ships (office.css)
//! office-preview version
//! ```
//!
//! A render writes `<cache>/<key>/` with `meta.json`, one JSON file per part (a docx section of
//! the body, an xlsx row window) holding `{"node": <TSP node>, "images": [{file, mime}]}`, and
//! the extracted images under `img/`. The key hashes the canonical path, size, mtime and the
//! renderer version, so an edited file renders again and an unchanged one is read back as is.

mod color;
mod css;
mod docx;
mod media;
mod node;
mod out;
mod pkg;
mod xlsx;
mod xml;

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Bumped whenever the output for the same input changes, so stale caches are not reused.
pub const RENDER_VERSION: u32 = 2;
/// Cache entries kept; older ones are deleted after each fresh render.
const CACHE_KEEP: usize = 24;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("render") => cmd_render(&args[1..]),
        Some("css") => {
            print!("{}", css::stylesheet());
            0
        }
        Some("version") => {
            println!("office-preview {} (render {})", env!("CARGO_PKG_VERSION"), RENDER_VERSION);
            0
        }
        _ => {
            eprintln!("usage: office-preview render <file> --cache <dir> | css | version");
            2
        }
    };
    std::process::exit(code);
}

fn cmd_render(args: &[String]) -> i32 {
    let mut file: Option<&str> = None;
    let mut cache: Option<&str> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--cache" => cache = it.next().map(String::as_str),
            _ if file.is_none() => file = Some(a),
            _ => {}
        }
    }
    let (Some(file), Some(cache)) = (file, cache) else {
        return fail("usage: office-preview render <file> --cache <dir>");
    };
    match render(Path::new(file), Path::new(cache)) {
        Ok((dir, meta)) => {
            println!("{}", json!({ "ok": true, "dir": dir.to_string_lossy(), "meta": meta }));
            0
        }
        Err(e) => fail(&e),
    }
}

fn fail(msg: &str) -> i32 {
    println!("{}", json!({ "ok": false, "error": msg }));
    1
}

/// Renders `file` into the cache (or finds it there) and returns the entry's directory and meta.
pub fn render(file: &Path, cache: &Path) -> Result<(PathBuf, Value), String> {
    let path = std::fs::canonicalize(file).map_err(|e| format!("无法打开 {}: {e}", file.display()))?;
    let md = std::fs::metadata(&path).map_err(|e| format!("无法读取 {}: {e}", path.display()))?;
    if !md.is_file() {
        return Err(format!("{} 不是文件", path.display()));
    }
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let key = format!(
        "{:016x}",
        fnv1a(format!("{}\0{}\0{}\0{}", path.display(), md.len(), mtime, RENDER_VERSION).as_bytes())
    );
    let dir = cache.join(&key);
    let meta_path = dir.join("meta.json");
    if let Ok(text) = std::fs::read_to_string(&meta_path) {
        if let Ok(meta) = serde_json::from_str::<Value>(&text) {
            // Touch so pruning keeps recently viewed entries.
            let _ = std::fs::write(dir.join(".used"), b"");
            return Ok((dir, meta));
        }
    }

    std::fs::create_dir_all(cache).map_err(|e| format!("无法创建缓存目录 {}: {e}", cache.display()))?;
    let tmp = cache.join(format!(".{key}.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("img")).map_err(|e| format!("无法写入缓存: {e}"))?;

    let result = render_into(&path, &tmp);
    let meta = match result {
        Ok(meta) => meta,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
    };
    std::fs::write(tmp.join("meta.json"), meta.to_string()).map_err(|e| format!("无法写入缓存: {e}"))?;
    let _ = std::fs::write(tmp.join(".used"), b"");
    let _ = std::fs::remove_dir_all(&dir);
    if std::fs::rename(&tmp, &dir).is_err() {
        // Another render of the same file won the race; use its result.
        let _ = std::fs::remove_dir_all(&tmp);
    }
    prune(cache, &key);
    Ok((dir, meta))
}

fn render_into(path: &Path, dir: &Path) -> Result<Value, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut package = pkg::Package::open(path)?;
    let mut out = out::Out::new(dir.to_path_buf());
    let kind = package.kind();
    let mut meta = match kind {
        pkg::Kind::Docx => docx::render(&mut package, &mut out)?,
        pkg::Kind::Xlsx => xlsx::render(&mut package, &mut out)?,
    };
    if let Value::Object(m) = &mut meta {
        m.insert("v".into(), json!(RENDER_VERSION));
        m.insert("name".into(), json!(name));
        m.insert("path".into(), json!(path.to_string_lossy()));
    }
    Ok(meta)
}

/// Deletes the least recently used cache entries beyond `CACHE_KEEP`, never `keep`.
fn prune(cache: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(cache) else { return };
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if !p.is_dir() || name == keep {
            continue;
        }
        if name.starts_with('.') {
            // A temporary directory of a render that died; old ones are garbage.
            let old = std::fs::metadata(&p)
                .and_then(|m| m.modified())
                .map(|t| t.elapsed().map(|d| d.as_secs() > 3600).unwrap_or(false))
                .unwrap_or(false);
            if old {
                let _ = std::fs::remove_dir_all(&p);
            }
            continue;
        }
        let used = std::fs::metadata(p.join(".used"))
            .and_then(|m| m.modified())
            .unwrap_or(UNIX_EPOCH);
        dirs.push((used, p));
    }
    if dirs.len() + 1 <= CACHE_KEEP {
        return;
    }
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, p) in dirs.into_iter().skip(CACHE_KEEP - 1) {
        let _ = std::fs::remove_dir_all(p);
    }
}

pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
