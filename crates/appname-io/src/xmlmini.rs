// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.4 (`stack.xml`); DD ⑫. A minimal, bounded XML-SUBSET
//! reader/writer for ORA's `stack.xml` — elements, attributes,
//! self-closing tags, comments, the XML declaration, processing
//! instructions, and the five predefined entities plus numeric character
//! references. DTDs/CDATA answer typed (`UnsupportedFeature`) — a layer
//! stack has no business carrying either, and rejecting them removes the
//! classic XML attack surface (entity expansion, external refs) outright.
//!
//! In-crate rather than a crate election because `quick-xml` (the ES §1.3
//! suggestion) has NO deps record — flagged to O-BUILD in the lane log;
//! if the record lands, this module shrinks to a shim. Untrusted input
//! discipline: bounded depth and node count, byte-indexed scanning, typed
//! errors, no panics (the D-series bar).

use crate::error::CodecError;

/// Depth bound (a hand-built hostile stack.xml, not a real layer tree).
const MAX_DEPTH: usize = 64;
/// Node bound (an ORA stack with more elements than this is a bomb).
const MAX_NODES: usize = 100_000;

/// One parsed element: name, attributes in DOCUMENT order, child elements
/// in DOCUMENT order (text content is skipped — `stack.xml` carries none).
#[derive(Debug, Clone)]
pub struct Elem {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Elem>,
}

impl Elem {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Elem> {
        self.children.iter().filter(move |c| c.name == name)
    }
}

fn bad(detail: impl std::fmt::Display) -> CodecError {
    CodecError::corrupt(format!("stack.xml: {detail}"))
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    nodes: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn starts(&self, s: &[u8]) -> bool {
        self.b[self.i..].starts_with(s)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.i += 1;
        }
    }

    /// Advance past `needle`, erroring if absent.
    fn skip_past(&mut self, needle: &[u8], what: &str) -> Result<(), CodecError> {
        match self.b[self.i..]
            .windows(needle.len())
            .position(|w| w == needle)
        {
            Some(p) => {
                self.i += p + needle.len();
                Ok(())
            }
            None => Err(bad(format!("unterminated {what}"))),
        }
    }

    fn name(&mut self) -> Result<String, CodecError> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || matches!(c, b':' | b'_' | b'-' | b'.') {
                self.i += 1;
            } else {
                break;
            }
        }
        if self.i == start {
            return Err(bad(format!("expected a name at byte {start}")));
        }
        String::from_utf8(self.b[start..self.i].to_vec()).map_err(bad)
    }

    fn attr_value(&mut self) -> Result<String, CodecError> {
        let quote = self.peek().ok_or_else(|| bad("truncated attribute"))?;
        if quote != b'"' && quote != b'\'' {
            return Err(bad("attribute value must be quoted"));
        }
        self.i += 1;
        let start = self.i;
        while let Some(c) = self.peek() {
            if c == quote {
                let raw = &self.b[start..self.i];
                self.i += 1;
                return unescape(raw);
            }
            if c == b'<' {
                return Err(bad("raw '<' inside an attribute value"));
            }
            self.i += 1;
        }
        Err(bad("unterminated attribute value"))
    }

    /// Skip misc content between elements (text, comments, PIs).
    fn skip_misc(&mut self) -> Result<(), CodecError> {
        loop {
            self.skip_ws();
            if self.starts(b"<!--") {
                self.skip_past(b"-->", "comment")?;
            } else if self.starts(b"<?") {
                self.skip_past(b"?>", "processing instruction")?;
            } else if self.starts(b"<![") {
                return Err(CodecError::unsupported("ora.stack_xml.cdata"));
            } else if self.starts(b"<!") {
                return Err(CodecError::unsupported("ora.stack_xml.dtd"));
            } else if matches!(self.peek(), Some(c) if c != b'<') {
                // Ignorable text content (stack.xml defines none).
                self.i += 1;
            } else {
                return Ok(());
            }
        }
    }

    fn elem(&mut self, depth: usize) -> Result<Elem, CodecError> {
        if depth > MAX_DEPTH {
            return Err(bad(format!("element depth exceeds {MAX_DEPTH}")));
        }
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(bad(format!("element count exceeds {MAX_NODES}")));
        }
        if self.peek() != Some(b'<') {
            return Err(bad("expected '<'"));
        }
        self.i += 1;
        let name = self.name()?;
        let mut attrs = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'/') => {
                    self.i += 1;
                    if self.peek() != Some(b'>') {
                        return Err(bad("malformed self-closing tag"));
                    }
                    self.i += 1;
                    return Ok(Elem {
                        name,
                        attrs,
                        children: Vec::new(),
                    });
                }
                Some(b'>') => {
                    self.i += 1;
                    break;
                }
                Some(_) => {
                    let attr_name = self.name()?;
                    self.skip_ws();
                    if self.peek() != Some(b'=') {
                        return Err(bad(format!("attribute `{attr_name}` missing '='")));
                    }
                    self.i += 1;
                    self.skip_ws();
                    let value = self.attr_value()?;
                    if attrs.iter().any(|(n, _)| *n == attr_name) {
                        return Err(bad(format!("duplicate attribute `{attr_name}`")));
                    }
                    attrs.push((attr_name, value));
                }
                None => return Err(bad("truncated tag")),
            }
        }
        // Children until the matching close tag.
        let mut children = Vec::new();
        loop {
            self.skip_misc()?;
            if self.starts(b"</") {
                self.i += 2;
                let close = self.name()?;
                if close != name {
                    return Err(bad(format!("mismatched close tag `{close}` for `{name}`")));
                }
                self.skip_ws();
                if self.peek() != Some(b'>') {
                    return Err(bad("malformed close tag"));
                }
                self.i += 1;
                return Ok(Elem {
                    name,
                    attrs,
                    children,
                });
            }
            if self.peek().is_none() {
                return Err(bad(format!("unterminated element `{name}`")));
            }
            children.push(self.elem(depth + 1)?);
        }
    }
}

/// Decode the five predefined entities + numeric character references.
fn unescape(raw: &[u8]) -> Result<String, CodecError> {
    let s = std::str::from_utf8(raw).map_err(bad)?;
    if !s.contains('&') {
        return Ok(s.to_string());
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let end = rest.find(';').ok_or_else(|| bad("unterminated entity"))?;
        let ent = &rest[1..end];
        match ent {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                let cp = u32::from_str_radix(&ent[2..], 16).map_err(bad)?;
                out.push(char::from_u32(cp).ok_or_else(|| bad("invalid character reference"))?);
            }
            _ if ent.starts_with('#') => {
                let cp = ent[1..].parse::<u32>().map_err(bad)?;
                out.push(char::from_u32(cp).ok_or_else(|| bad("invalid character reference"))?);
            }
            other => return Err(bad(format!("unknown entity `&{other};`"))),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Escape a value for an attribute position (double-quoted).
pub fn escape_attr(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Parse one document: optional BOM/decl/comments, one root element.
pub fn parse(bytes: &[u8]) -> Result<Elem, CodecError> {
    let mut p = Parser {
        b: bytes,
        i: 0,
        nodes: 0,
    };
    if p.starts(&[0xEF, 0xBB, 0xBF]) {
        p.i += 3;
    }
    p.skip_misc()?;
    let root = p.elem(0)?;
    p.skip_misc()?;
    if p.peek().is_some() {
        return Err(bad("content after the root element"));
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_stack_xml_shape() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<image w="4" h="3" xmlns:appname="https://appname.org/ns/ora/1">
  <!-- layer stack -->
  <stack>
    <layer name="A &amp; B" src="data/l0.png" opacity="0.5" visibility="hidden"/>
    <stack name="G" isolation="auto">
      <layer name="inner" src="data/l1.png"/>
    </stack>
  </stack>
</image>"#;
        let root = parse(xml).unwrap();
        assert_eq!(root.name, "image");
        assert_eq!(root.attr("w"), Some("4"));
        let stack = root.children_named("stack").next().unwrap();
        assert_eq!(stack.children.len(), 2);
        assert_eq!(stack.children[0].attr("name"), Some("A & B"));
        assert_eq!(stack.children[1].attr("isolation"), Some("auto"));
        assert_eq!(stack.children[1].children[0].attr("name"), Some("inner"));
    }

    #[test]
    fn rejects_dtd_and_cdata_typed() {
        let dtd = b"<!DOCTYPE image [<!ENTITY x \"y\">]><image/>";
        assert_eq!(
            parse(dtd).err().unwrap().message_id(),
            "error.codec.unsupported_feature"
        );
        let cdata = b"<image><![CDATA[nope]]></image>";
        assert_eq!(
            parse(cdata).err().unwrap().message_id(),
            "error.codec.unsupported_feature"
        );
    }

    #[test]
    fn garbage_is_typed_never_panics() {
        for junk in [
            &b"<image"[..],
            b"<image a=b/>",
            b"<image><layer></image>",
            b"<image v=\"&nope;\"/>",
            b"not xml at all",
            b"<image attr=\"unterminated/>",
            b"</image>",
        ] {
            let err = parse(junk).expect_err("must reject");
            assert!(err.message_id().starts_with("error.codec."), "{err}");
        }
    }

    #[test]
    fn escape_round_trips() {
        let ugly = "a<b>&\"c'";
        let xml = format!("<x v=\"{}\"/>", escape_attr(ugly));
        let root = parse(xml.as_bytes()).unwrap();
        assert_eq!(root.attr("v"), Some(ugly));
    }
}
