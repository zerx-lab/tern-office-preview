//! Writes a render's files: parts (`<name>.json`) and pictures (`img/<n>.<ext>`).

use crate::media;
use crate::node::Node;
use crate::pkg::Package;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

/// A picture written to the cache, as renderers reference it.
#[derive(Clone, Debug)]
pub struct Media {
    /// Cache-relative file (`img/3.png`), the `src` of an `image` node.
    pub file: String,
    /// Natural size in px (0 when unknown, e.g. SVG).
    pub w: u32,
    pub h: u32,
}

pub struct Out {
    dir: PathBuf,
    /// Part name → written picture, or the reason it can't be shown.
    media: HashMap<String, Result<Media, String>>,
    mimes: HashMap<String, &'static str>,
    next: usize,
}

impl Out {
    pub fn new(dir: PathBuf) -> Out {
        Out { dir, media: HashMap::new(), mimes: HashMap::new(), next: 0 }
    }

    /// Extracts picture part `part` once and returns where it was written, or why it wasn't.
    pub fn media(&mut self, pkg: &mut Package, part: &str) -> Result<Media, String> {
        if let Some(m) = self.media.get(part) {
            return m.clone();
        }
        let res = match pkg.bytes(part) {
            None => Err("图片缺失".to_string()),
            Some(bytes) => match media::prepare(bytes) {
                Err(why) => Err(why.to_string()),
                Ok(pic) => {
                    self.next += 1;
                    let file = format!("img/{}.{}", self.next, pic.ext);
                    match std::fs::write(self.dir.join(&file), &pic.bytes) {
                        Ok(()) => {
                            self.mimes.insert(file.clone(), pic.mime);
                            Ok(Media { file, w: pic.w, h: pic.h })
                        }
                        Err(e) => Err(format!("写入图片失败: {e}")),
                    }
                }
            },
        };
        self.media.insert(part.to_string(), res.clone());
        res
    }

    /// Writes part `name` (`part-0`, `s1-3`) as `{"node": …, "images": [{file, mime}]}`.
    pub fn write_part(&self, name: &str, node: Node) -> Result<(), String> {
        let mut images = Vec::new();
        collect_images(&node, &mut images);
        images.sort();
        images.dedup();
        let images: Vec<Value> = images
            .into_iter()
            .map(|f| json!({ "file": f, "mime": self.mimes.get(&f).copied().unwrap_or("image/png") }))
            .collect();
        let v = json!({ "node": node.into_value(), "images": images });
        std::fs::write(self.dir.join(format!("{name}.json")), v.to_string()).map_err(|e| format!("无法写入缓存: {e}"))
    }

    /// Like `write_part`, for a node already serialized as JSON (`node_json`) that uses the
    /// pictures `images` (cache-relative files). Lets a renderer stream huge parts without
    /// building a `Node` tree.
    pub fn write_part_raw(&self, name: &str, node_json: &str, images: &[String]) -> Result<(), String> {
        let mut imgs: Vec<&String> = images.iter().collect();
        imgs.sort();
        imgs.dedup();
        let list: Vec<Value> = imgs
            .into_iter()
            .map(|f| json!({ "file": f, "mime": self.mimes.get(f).copied().unwrap_or("image/png") }))
            .collect();
        let mut s = String::with_capacity(node_json.len() + 64);
        s.push_str("{\"node\":");
        s.push_str(node_json);
        s.push_str(",\"images\":");
        s.push_str(&Value::Array(list).to_string());
        s.push('}');
        std::fs::write(self.dir.join(format!("{name}.json")), s).map_err(|e| format!("无法写入缓存: {e}"))
    }
}

fn collect_images(n: &Node, out: &mut Vec<String>) {
    if n.kind == "image" {
        if let Some(Value::String(s)) = n.props.get("src") {
            out.push(s.clone());
        }
    }
    for c in &n.children {
        collect_images(c, out);
    }
}
