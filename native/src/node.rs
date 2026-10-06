//! TSP view nodes (`{k, p, c}`) as JSON, the shape Tern's `tern.ui` builders produce.

use serde_json::{Map, Value};

/// A node under construction. `into_value` drops empty props and children.
#[derive(Debug, Default, Clone)]
pub struct Node {
    pub kind: &'static str,
    pub props: Map<String, Value>,
    pub children: Vec<Node>,
}

impl Node {
    /// An `el` node of HTML tag `tag`.
    pub fn el(tag: &str) -> Node {
        let mut n = Node { kind: "el", ..Default::default() };
        n.props.insert("tag".into(), Value::String(tag.into()));
        n
    }

    pub fn kind(kind: &'static str) -> Node {
        Node { kind, ..Default::default() }
    }

    pub fn class(mut self, class: &str) -> Node {
        self.set_class(class);
        self
    }

    /// Sets `class`, trimming spaces; an empty class sets nothing.
    pub fn set_class(&mut self, class: &str) {
        let c = class.trim();
        if !c.is_empty() {
            self.props.insert("class".into(), Value::String(c.into()));
        }
    }

    pub fn text(mut self, text: impl Into<String>) -> Node {
        let t: String = text.into();
        if !t.is_empty() {
            self.props.insert("text".into(), Value::String(t));
        }
        self
    }

    pub fn prop(mut self, key: &str, v: impl Into<Value>) -> Node {
        self.props.insert(key.into(), v.into());
        self
    }

    pub fn child(mut self, c: Node) -> Node {
        self.children.push(c);
        self
    }

    pub fn push(&mut self, c: Node) {
        self.children.push(c);
    }

    pub fn has_text(&self) -> bool {
        self.props.get("text").is_some_and(|t| t.as_str().is_some_and(|s| !s.is_empty()))
    }

    /// Rough count of nodes in the tree, for splitting output into parts.
    pub fn weight(&self) -> usize {
        1 + self.children.iter().map(Node::weight).sum::<usize>()
    }

    pub fn into_value(self) -> Value {
        let mut m = Map::new();
        m.insert("k".into(), Value::String(self.kind.into()));
        if !self.props.is_empty() {
            m.insert("p".into(), Value::Object(self.props));
        }
        if !self.children.is_empty() {
            m.insert("c".into(), Value::Array(self.children.into_iter().map(Node::into_value).collect()));
        }
        Value::Object(m)
    }
}

/// An `image` node for an extracted picture: `src` is the cache-relative file the plugin turns
/// into a blob; `w`/`h` are its display size in px.
pub fn image(src: &str, w: f64, h: f64, alt: &str) -> Node {
    let mut n = Node::kind("image").prop("src", src);
    if w > 0.0 {
        n = n.prop("w", w.round());
    }
    if h > 0.0 {
        n = n.prop("h", h.round());
    }
    if !alt.is_empty() {
        n = n.prop("alt", alt);
    }
    n
}

/// A dashed placeholder for something the preview can't draw (an EMF picture, a chart).
pub fn placeholder(label: &str) -> Node {
    Node::el("span").class("ph").text(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_parts_are_dropped() {
        let v = Node::el("div").class(" ").text("").into_value();
        assert_eq!(v.to_string(), r#"{"k":"el","p":{"tag":"div"}}"#);
        let v = Node::el("div").class("w16 gs2").child(Node::el("span").text("x")).into_value();
        assert_eq!(
            v.to_string(),
            r#"{"k":"el","p":{"tag":"div","class":"w16 gs2"},"c":[{"k":"el","p":{"tag":"span","text":"x"}}]}"#
        );
    }
}
