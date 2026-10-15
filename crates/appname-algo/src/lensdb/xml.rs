// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Minimal own-written XML subset parser for the lens profile database.
//!
//! Scope (rule: exactly what the vendored DB corpus observably uses, per
//! the first-hand survey of `lensfun-database.dtd` / `.xsd` and the 56
//! profile `*.xml` files at the census pin, plus the standard predefined
//! entities):
//!
//! - an optional leading UTF-8 BOM, accepted and skipped (not present
//!   in the corpus; tolerated because the data dir is user-replaceable
//!   and Windows editors commonly write one);
//! - optional leading XML declaration `<?xml ... ?>` (not present in the
//!   corpus profile files, which open with a DOCTYPE line; tolerated
//!   because the data dir is user-replaceable);
//! - optional `<!DOCTYPE name SYSTEM "...">` WITHOUT an internal subset
//!   (`[` inside DOCTYPE is rejected honestly — internal subsets can
//!   declare entities this parser will not process);
//! - one root element; elements with double- or single-quoted attributes,
//!   self-closing forms, nested elements, text content, and comments
//!   (`<!-- ... -->`, terminated at the first `-->`);
//! - the five predefined entities `&amp; &lt; &gt; &apos; &quot;`
//!   (the corpus uses only `&amp;`, 257 occurrences surveyed);
//! - UTF-8 text content (the corpus carries `lang="ru"`/`"de"` names).
//!
//! Everything else is REJECTED with a typed [`XmlError`] carrying byte
//! offset + 1-based line/column: CDATA sections, processing instructions
//! in content, numeric character references, unknown entities, internal
//! DTD subsets, duplicate attributes, mismatched or unclosed tags,
//! trailing content after the document element, and — per XML 1.0's
//! `Char` production — NUL and every other C0 control character except
//! tab/LF/CR in text and attribute values (NUL especially is a
//! string-handling hazard for downstream display surfaces; comment
//! bodies are skipped uninspected and dropped, so control characters
//! inside comments are accepted — documented, since comments never
//! reach any consumer). The data directory is
//! user-replaceable (ES Part 43 posture; ALGORITHMS.md §21 data rule), so
//! malformed input must produce `Err`, never a panic or UB.
//!
//! Hardening posture: the parser is fully ITERATIVE (explicit stack, no
//! recursion — stack safety on user-replaced data) with an explicit
//! nesting bound [`MAX_DEPTH`]; all cursor arithmetic is bounds-checked
//! indexing over the input slice (no length arithmetic that can
//! overflow). Determinism (D-48): the parse is a pure function of the
//! input string — one pass, fixed order, no environment access.
//!
//! Zero-copy where reasonable: element/attribute names are `&str` slices
//! of the input; text and attribute values are `Cow<str>` (borrowed
//! unless entity decoding forces an owned copy).

use std::borrow::Cow;
use std::fmt;

/// Maximum element nesting depth (explicit bound; the corpus needs 3).
pub const MAX_DEPTH: usize = 64;

/// Longest accepted entity body between `&` and `;` (`quot` is 4).
const MAX_ENTITY_LEN: usize = 8;

/// What went wrong, as a closed set of malformed-input classes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XmlErrorKind {
    /// Input ended inside the named construct.
    UnexpectedEof(&'static str),
    /// A construct is present but ill-formed (static description).
    Malformed(&'static str),
    /// Close tag does not match the open element.
    MismatchedClose {
        /// Name of the innermost open element.
        expected: String,
        /// Name found in the close tag.
        found: String,
    },
    /// `&name;` with a name outside the five predefined entities.
    UnknownEntity(String),
    /// A construct outside the supported subset (CDATA, PI in content,
    /// numeric character reference, DOCTYPE internal subset).
    Unsupported(&'static str),
    /// Element nesting exceeded [`MAX_DEPTH`].
    DepthExceeded(usize),
    /// The same attribute name appeared twice on one element.
    DuplicateAttribute(String),
}

/// Typed parse error with position info (byte offset + 1-based line and
/// 1-based byte column within that line).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XmlError {
    /// Malformed-input class.
    pub kind: XmlErrorKind,
    /// Byte offset into the input where the error was detected.
    pub offset: usize,
    /// 1-based line number.
    pub line: usize,
    /// 1-based byte column within the line.
    pub col: usize,
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}: ", self.line, self.col)?;
        match &self.kind {
            XmlErrorKind::UnexpectedEof(what) => {
                write!(f, "unexpected end of input inside {what}")
            }
            XmlErrorKind::Malformed(what) => write!(f, "malformed XML: {what}"),
            XmlErrorKind::MismatchedClose { expected, found } => {
                write!(
                    f,
                    "close tag </{found}> does not match open element <{expected}>"
                )
            }
            XmlErrorKind::UnknownEntity(name) => write!(f, "unknown entity &{name};"),
            XmlErrorKind::Unsupported(what) => {
                write!(f, "unsupported XML construct: {what}")
            }
            XmlErrorKind::DepthExceeded(bound) => {
                write!(f, "element nesting exceeds the depth bound {bound}")
            }
            XmlErrorKind::DuplicateAttribute(name) => {
                write!(f, "duplicate attribute {name:?}")
            }
        }
    }
}

/// 1-based (line, byte-column) of a byte offset (error/report helper).
pub fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let clamped = offset.min(src.len());
    let prefix = &src.as_bytes()[..clamped];
    let line = 1 + prefix.iter().filter(|&&b| b == b'\n').count();
    let line_start = prefix
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |p| p + 1);
    (line, clamped - line_start + 1)
}

/// One attribute (`name="value"`); value is entity-decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attr<'a> {
    /// Attribute name (slice of the input).
    pub name: &'a str,
    /// Decoded attribute value.
    pub value: Cow<'a, str>,
    /// Byte offset of the attribute name (for schema-layer errors).
    pub offset: usize,
}

/// An element node with attributes and children, in document order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element<'a> {
    /// Element name (slice of the input).
    pub name: &'a str,
    /// Attributes in document order.
    pub attrs: Vec<Attr<'a>>,
    /// Child nodes in document order (text nodes preserved verbatim,
    /// including whitespace-only runs; comments are dropped).
    pub children: Vec<Node<'a>>,
    /// Byte offset of the `<` of the start tag.
    pub offset: usize,
}

/// A child node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node<'a> {
    /// Nested element.
    Element(Element<'a>),
    /// Decoded character data (verbatim; schema layer trims).
    Text(Cow<'a, str>),
}

impl<'a> Element<'a> {
    /// Attribute value by name, if present.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.as_ref())
    }

    /// Child elements (text nodes skipped).
    pub fn child_elements(&self) -> impl Iterator<Item = &Element<'a>> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// Concatenated text content of DIRECT text children, trimmed.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for n in &self.children {
            if let Node::Text(t) = n {
                out.push_str(t);
            }
        }
        out.trim().to_owned()
    }
}

/// Parse a complete document; returns the root element.
pub fn parse_document(input: &str) -> Result<Element<'_>, XmlError> {
    Parser {
        src: input,
        b: input.as_bytes(),
        i: 0,
    }
    .document()
}

struct Parser<'a> {
    src: &'a str,
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn err<T>(&self, kind: XmlErrorKind, offset: usize) -> Result<T, XmlError> {
        let (line, col) = line_col(self.src, offset);
        Err(XmlError {
            kind,
            offset,
            line,
            col,
        })
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn starts_with(&self, pat: &[u8]) -> bool {
        self.b[self.i..].starts_with(pat)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.i += 1;
        }
    }

    /// Advance past `pat` starting at `self.i`, or report `what`.
    fn expect(&mut self, pat: &[u8], what: &'static str) -> Result<(), XmlError> {
        if self.starts_with(pat) {
            self.i += pat.len();
            Ok(())
        } else if self.i >= self.b.len() {
            self.err(XmlErrorKind::UnexpectedEof(what), self.i)
        } else {
            self.err(XmlErrorKind::Malformed(what), self.i)
        }
    }

    /// Scan forward to just past `terminator`, or EOF-error.
    fn skip_until(&mut self, terminator: &[u8], what: &'static str) -> Result<(), XmlError> {
        let start = self.i;
        while self.i < self.b.len() {
            if self.starts_with(terminator) {
                self.i += terminator.len();
                return Ok(());
            }
            self.i += 1;
        }
        self.err(XmlErrorKind::UnexpectedEof(what), start)
    }

    fn is_name_start(b: u8) -> bool {
        b.is_ascii_alphabetic() || b == b'_'
    }

    fn is_name_char(b: u8) -> bool {
        b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':')
    }

    /// C0 control byte forbidden by XML 1.0's `Char` production
    /// (everything below 0x20 except tab, LF, CR).
    fn is_forbidden_control(b: u8) -> bool {
        b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r')
    }

    /// Parse an element or attribute name (ASCII subset per the corpus).
    fn name(&mut self) -> Result<&'a str, XmlError> {
        let start = self.i;
        match self.peek() {
            Some(c) if Self::is_name_start(c) => self.i += 1,
            Some(_) => return self.err(XmlErrorKind::Malformed("expected a name"), self.i),
            None => return self.err(XmlErrorKind::UnexpectedEof("name"), self.i),
        }
        while matches!(self.peek(), Some(c) if Self::is_name_char(c)) {
            self.i += 1;
        }
        Ok(&self.src[start..self.i])
    }

    /// Decode entities in `raw` (slice starting at `base` in the input).
    fn decode(&self, raw: &'a str, base: usize) -> Result<Cow<'a, str>, XmlError> {
        if !raw.contains('&') {
            return Ok(Cow::Borrowed(raw));
        }
        let bytes = raw.as_bytes();
        let mut out = String::with_capacity(raw.len());
        let mut i = 0usize;
        let mut plain_start = 0usize;
        while i < bytes.len() {
            if bytes[i] != b'&' {
                i += 1;
                continue;
            }
            out.push_str(&raw[plain_start..i]);
            let ent_start = i + 1;
            let mut j = ent_start;
            while j < bytes.len() && bytes[j] != b';' && j - ent_start <= MAX_ENTITY_LEN {
                j += 1;
            }
            if j >= bytes.len() || bytes[j] != b';' {
                return self.err(
                    XmlErrorKind::Malformed("unterminated entity reference"),
                    base + i,
                );
            }
            let ent = &raw[ent_start..j];
            if ent.starts_with('#') {
                return self.err(
                    XmlErrorKind::Unsupported("numeric character reference"),
                    base + i,
                );
            }
            match ent {
                "amp" => out.push('&'),
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "apos" => out.push('\''),
                "quot" => out.push('"'),
                other => return self.err(XmlErrorKind::UnknownEntity(other.to_owned()), base + i),
            }
            i = j + 1;
            plain_start = i;
        }
        out.push_str(&raw[plain_start..]);
        Ok(Cow::Owned(out))
    }

    /// Prolog: optional UTF-8 BOM, optional XML declaration, then
    /// comments and one optional DOCTYPE (no internal subset), until
    /// the root start tag.
    fn prolog(&mut self) -> Result<(), XmlError> {
        // Accept-and-skip a leading BOM (user-replaceable data dir:
        // Windows editors commonly write one). Only at offset 0 —
        // U+FEFF anywhere else is ordinary character data (ZWNBSP).
        if self.starts_with(b"\xEF\xBB\xBF") {
            self.i += 3;
        }
        self.skip_ws();
        if self.starts_with(b"<?xml") {
            self.i += 5;
            self.skip_until(b"?>", "XML declaration")?;
        }
        let mut seen_doctype = false;
        loop {
            self.skip_ws();
            if self.starts_with(b"<!--") {
                self.i += 4;
                self.skip_until(b"-->", "comment")?;
            } else if self.starts_with(b"<!DOCTYPE") {
                if seen_doctype {
                    return self.err(XmlErrorKind::Malformed("second DOCTYPE"), self.i);
                }
                seen_doctype = true;
                let start = self.i;
                self.i += 9;
                loop {
                    match self.peek() {
                        Some(b'>') => {
                            self.i += 1;
                            break;
                        }
                        Some(b'[') => {
                            return self
                                .err(XmlErrorKind::Unsupported("DOCTYPE internal subset"), self.i)
                        }
                        Some(_) => self.i += 1,
                        None => return self.err(XmlErrorKind::UnexpectedEof("DOCTYPE"), start),
                    }
                }
            } else {
                return Ok(());
            }
        }
    }

    /// Start tag after `<`: name + attributes; returns (element, closed).
    fn open_tag(&mut self, offset: usize) -> Result<(Element<'a>, bool), XmlError> {
        let name = self.name()?;
        let mut attrs: Vec<Attr<'a>> = Vec::new();
        loop {
            let before = self.i;
            self.skip_ws();
            match self.peek() {
                Some(b'>') => {
                    self.i += 1;
                    return Ok((
                        Element {
                            name,
                            attrs,
                            children: Vec::new(),
                            offset,
                        },
                        false,
                    ));
                }
                Some(b'/') => {
                    self.i += 1;
                    self.expect(b">", "self-closing tag end")?;
                    return Ok((
                        Element {
                            name,
                            attrs,
                            children: Vec::new(),
                            offset,
                        },
                        true,
                    ));
                }
                Some(_) => {
                    if before == self.i {
                        // No whitespace consumed before this attribute.
                        return self.err(
                            XmlErrorKind::Malformed("expected whitespace before attribute"),
                            self.i,
                        );
                    }
                    let attr_offset = self.i;
                    let attr_name = self.name()?;
                    self.skip_ws();
                    self.expect(b"=", "attribute equals sign")?;
                    self.skip_ws();
                    let quote = match self.peek() {
                        Some(q @ (b'"' | b'\'')) => q,
                        Some(_) => {
                            return self.err(
                                XmlErrorKind::Malformed("attribute value must be quoted"),
                                self.i,
                            )
                        }
                        None => {
                            return self.err(XmlErrorKind::UnexpectedEof("attribute value"), self.i)
                        }
                    };
                    self.i += 1;
                    let vstart = self.i;
                    loop {
                        match self.peek() {
                            Some(q) if q == quote => break,
                            Some(b'<') => {
                                return self.err(
                                    XmlErrorKind::Malformed("raw '<' in attribute value"),
                                    self.i,
                                )
                            }
                            Some(c) if Self::is_forbidden_control(c) => {
                                return self.err(
                                    XmlErrorKind::Malformed(
                                        "control character forbidden by XML 1.0",
                                    ),
                                    self.i,
                                )
                            }
                            Some(_) => self.i += 1,
                            None => {
                                return self
                                    .err(XmlErrorKind::UnexpectedEof("attribute value"), vstart)
                            }
                        }
                    }
                    let raw = &self.src[vstart..self.i];
                    self.i += 1; // closing quote
                    if attrs.iter().any(|a| a.name == attr_name) {
                        return self.err(
                            XmlErrorKind::DuplicateAttribute(attr_name.to_owned()),
                            attr_offset,
                        );
                    }
                    let value = self.decode(raw, vstart)?;
                    attrs.push(Attr {
                        name: attr_name,
                        value,
                        offset: attr_offset,
                    });
                }
                None => return self.err(XmlErrorKind::UnexpectedEof("start tag"), offset),
            }
        }
    }

    /// The document: prolog, one element tree (iterative), trailing misc.
    fn document(&mut self) -> Result<Element<'a>, XmlError> {
        self.prolog()?;
        let root_offset = self.i;
        if self.peek() != Some(b'<') {
            return self.err(
                XmlErrorKind::Malformed("expected the document element"),
                self.i,
            );
        }
        self.i += 1;
        let (root, closed) = self.open_tag(root_offset)?;
        let root = if closed { root } else { self.content(root)? };
        // Trailing: whitespace and comments only.
        loop {
            self.skip_ws();
            if self.starts_with(b"<!--") {
                self.i += 4;
                self.skip_until(b"-->", "comment")?;
            } else if self.i >= self.b.len() {
                return Ok(root);
            } else {
                return self.err(
                    XmlErrorKind::Malformed("content after the document element"),
                    self.i,
                );
            }
        }
    }

    /// Iterative content loop for a non-self-closed element.
    fn content(&mut self, open: Element<'a>) -> Result<Element<'a>, XmlError> {
        let mut stack: Vec<Element<'a>> = vec![open];
        loop {
            match self.peek() {
                None => {
                    let innermost = stack.last().expect("stack is never empty here");
                    return self.err(
                        XmlErrorKind::UnexpectedEof("unclosed element"),
                        innermost.offset,
                    );
                }
                Some(b'<') => {
                    if self.starts_with(b"<!--") {
                        self.i += 4;
                        self.skip_until(b"-->", "comment")?;
                    } else if self.starts_with(b"<![") {
                        return self.err(XmlErrorKind::Unsupported("CDATA section"), self.i);
                    } else if self.starts_with(b"<!") {
                        return self.err(
                            XmlErrorKind::Unsupported("markup declaration in content"),
                            self.i,
                        );
                    } else if self.starts_with(b"<?") {
                        return self.err(
                            XmlErrorKind::Unsupported("processing instruction in content"),
                            self.i,
                        );
                    } else if self.starts_with(b"</") {
                        let close_offset = self.i;
                        self.i += 2;
                        let close_name = self.name()?;
                        self.skip_ws();
                        self.expect(b">", "close tag end")?;
                        let done = stack.pop().expect("stack is never empty here");
                        if done.name != close_name {
                            return self.err(
                                XmlErrorKind::MismatchedClose {
                                    expected: done.name.to_owned(),
                                    found: close_name.to_owned(),
                                },
                                close_offset,
                            );
                        }
                        match stack.last_mut() {
                            Some(parent) => parent.children.push(Node::Element(done)),
                            None => return Ok(done),
                        }
                    } else {
                        let offset = self.i;
                        self.i += 1;
                        if stack.len() >= MAX_DEPTH {
                            return self.err(XmlErrorKind::DepthExceeded(MAX_DEPTH), offset);
                        }
                        let (elem, closed) = self.open_tag(offset)?;
                        if closed {
                            stack
                                .last_mut()
                                .expect("stack is never empty here")
                                .children
                                .push(Node::Element(elem));
                        } else {
                            stack.push(elem);
                        }
                    }
                }
                Some(_) => {
                    let start = self.i;
                    while self.i < self.b.len() && self.b[self.i] != b'<' {
                        if Self::is_forbidden_control(self.b[self.i]) {
                            return self.err(
                                XmlErrorKind::Malformed("control character forbidden by XML 1.0"),
                                self.i,
                            );
                        }
                        self.i += 1;
                    }
                    let raw = &self.src[start..self.i];
                    let text = self.decode(raw, start)?;
                    stack
                        .last_mut()
                        .expect("stack is never empty here")
                        .children
                        .push(Node::Text(text));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind_of(r: Result<Element<'_>, XmlError>) -> XmlErrorKind {
        r.expect_err("expected a parse error").kind
    }

    #[test]
    fn well_formed_subset_parses() {
        let doc = "\
<!DOCTYPE lensdatabase SYSTEM \"lensfun-database.dtd\">
<lensdatabase version=\"2\">
    <!-- a comment with <angle brackets> and an address <x@example.org> -->
    <mount>
        <name>Canon EF</name>
        <compat>M42</compat>
    </mount>
    <lens>
        <model>Foo 10-22mm f/3.5 &amp; friends</model>
        <calibration>
            <distortion model=\"ptlens\" focal=\"10\" a=\"0.01\" b=\"-0.06\" c=\"0.05\"/>
        </calibration>
    </lens>
</lensdatabase>
<!-- trailing comment -->
";
        let root = parse_document(doc).expect("parse");
        assert_eq!(root.name, "lensdatabase");
        assert_eq!(root.attr("version"), Some("2"));
        let children: Vec<&Element<'_>> = root.child_elements().collect();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name, "mount");
        let mount_kids: Vec<&Element<'_>> = children[0].child_elements().collect();
        assert_eq!(mount_kids[0].text(), "Canon EF");
        let lens_kids: Vec<&Element<'_>> = children[1].child_elements().collect();
        assert_eq!(lens_kids[0].text(), "Foo 10-22mm f/3.5 & friends");
        let cal: Vec<&Element<'_>> = lens_kids[1].child_elements().collect();
        assert_eq!(cal[0].attr("model"), Some("ptlens"));
        assert_eq!(cal[0].attr("b"), Some("-0.06"));
    }

    #[test]
    fn xml_declaration_and_single_quotes_accepted() {
        let root = parse_document("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<a x='1'/>")
            .expect("parse");
        assert_eq!(root.name, "a");
        assert_eq!(root.attr("x"), Some("1"));
    }

    #[test]
    fn all_five_predefined_entities_decode() {
        let root =
            parse_document("<a t=\"&lt;&gt;&amp;&apos;&quot;\">&lt;x&gt;</a>").expect("parse");
        assert_eq!(root.attr("t"), Some("<>&'\""));
        assert_eq!(root.text(), "<x>");
    }

    #[test]
    fn utf8_text_preserved() {
        let root = parse_document("<m><name lang=\"ru\">Байонет K</name></m>").expect("parse");
        let name: Vec<&Element<'_>> = root.child_elements().collect();
        assert_eq!(name[0].text(), "Байонет K");
        assert_eq!(name[0].attr("lang"), Some("ru"));
    }

    #[test]
    fn whitespace_only_text_nodes_are_preserved_verbatim() {
        let root = parse_document("<a>\n  <b/>\n</a>").expect("parse");
        assert_eq!(root.children.len(), 3);
        assert!(matches!(&root.children[0], Node::Text(t) if t.as_ref() == "\n  "));
    }

    #[test]
    fn truncated_tag_is_unexpected_eof() {
        assert!(matches!(
            kind_of(parse_document("<a><b attr=\"1")),
            XmlErrorKind::UnexpectedEof(_)
        ));
    }

    #[test]
    fn unclosed_element_is_unexpected_eof() {
        assert!(matches!(
            kind_of(parse_document("<a><b></b>")),
            XmlErrorKind::UnexpectedEof("unclosed element")
        ));
    }

    #[test]
    fn mismatched_close_tag_reports_both_names() {
        match kind_of(parse_document("<a><b></c></a>")) {
            XmlErrorKind::MismatchedClose { expected, found } => {
                assert_eq!(expected, "b");
                assert_eq!(found, "c");
            }
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn unknown_entity_rejected() {
        assert_eq!(
            kind_of(parse_document("<a>&nbsp;</a>")),
            XmlErrorKind::UnknownEntity("nbsp".to_owned())
        );
    }

    #[test]
    fn numeric_character_reference_rejected() {
        assert_eq!(
            kind_of(parse_document("<a>&#65;</a>")),
            XmlErrorKind::Unsupported("numeric character reference")
        );
    }

    #[test]
    fn unterminated_entity_rejected() {
        assert!(matches!(
            kind_of(parse_document("<a>x &amp y</a>")),
            XmlErrorKind::Malformed("unterminated entity reference")
        ));
    }

    #[test]
    fn cdata_rejected() {
        assert_eq!(
            kind_of(parse_document("<a><![CDATA[x]]></a>")),
            XmlErrorKind::Unsupported("CDATA section")
        );
    }

    #[test]
    fn processing_instruction_in_content_rejected() {
        assert_eq!(
            kind_of(parse_document("<a><?pi x?></a>")),
            XmlErrorKind::Unsupported("processing instruction in content")
        );
    }

    #[test]
    fn doctype_internal_subset_rejected() {
        assert_eq!(
            kind_of(parse_document(
                "<!DOCTYPE a [<!ENTITY e \"x\">]>\n<a>&e;</a>"
            )),
            XmlErrorKind::Unsupported("DOCTYPE internal subset")
        );
    }

    #[test]
    fn duplicate_attribute_rejected() {
        assert_eq!(
            kind_of(parse_document("<a x=\"1\" x=\"2\"/>")),
            XmlErrorKind::DuplicateAttribute("x".to_owned())
        );
    }

    #[test]
    fn unquoted_attribute_value_rejected() {
        assert!(matches!(
            kind_of(parse_document("<a x=1/>")),
            XmlErrorKind::Malformed("attribute value must be quoted")
        ));
    }

    #[test]
    fn missing_whitespace_between_attributes_rejected() {
        assert!(matches!(
            kind_of(parse_document("<a x=\"1\"y=\"2\"/>")),
            XmlErrorKind::Malformed("expected whitespace before attribute")
        ));
    }

    #[test]
    fn raw_angle_in_attribute_value_rejected() {
        assert!(matches!(
            kind_of(parse_document("<a x=\"a<b\"/>")),
            XmlErrorKind::Malformed("raw '<' in attribute value")
        ));
    }

    #[test]
    fn content_after_document_element_rejected() {
        assert!(matches!(
            kind_of(parse_document("<a/><b/>")),
            XmlErrorKind::Malformed("content after the document element")
        ));
    }

    #[test]
    fn unclosed_comment_rejected() {
        assert!(matches!(
            kind_of(parse_document("<a><!-- never closed </a>")),
            XmlErrorKind::UnexpectedEof("comment")
        ));
    }

    #[test]
    fn leading_utf8_bom_is_accepted_and_skipped() {
        // Bare BOM + document.
        let root = parse_document("\u{FEFF}<a x=\"1\"/>").expect("parse");
        assert_eq!(root.name, "a");
        assert_eq!(root.attr("x"), Some("1"));
        // BOM + XML declaration + DOCTYPE (the Windows-editor shape).
        let root = parse_document(
            "\u{FEFF}<?xml version=\"1.0\"?>\n\
             <!DOCTYPE a SYSTEM \"x.dtd\">\n<a/>",
        )
        .expect("parse");
        assert_eq!(root.name, "a");
    }

    #[test]
    fn bom_then_garbage_still_reports_document_element() {
        // The BOM is consumed; the error names the real problem, not
        // a misleading complaint about the BOM bytes themselves.
        assert!(matches!(
            kind_of(parse_document("\u{FEFF}hello <a/>")),
            XmlErrorKind::Malformed("expected the document element")
        ));
    }

    #[test]
    fn bom_not_at_start_is_ordinary_character_data() {
        // U+FEFF inside content is ZWNBSP text, not a BOM.
        let root = parse_document("<a>\u{FEFF}x</a>").expect("parse");
        assert_eq!(root.children.len(), 1);
        assert!(matches!(&root.children[0], Node::Text(t) if t.as_ref() == "\u{FEFF}x"));
    }

    #[test]
    fn nul_in_text_rejected() {
        assert!(matches!(
            kind_of(parse_document("<a>x\u{0}y</a>")),
            XmlErrorKind::Malformed("control character forbidden by XML 1.0")
        ));
    }

    #[test]
    fn nul_in_attribute_value_rejected() {
        assert!(matches!(
            kind_of(parse_document("<a x=\"a\u{0}b\"/>")),
            XmlErrorKind::Malformed("control character forbidden by XML 1.0")
        ));
    }

    #[test]
    fn other_c0_controls_in_text_and_attributes_rejected() {
        // XML 1.0 Char forbids all of C0 except tab/LF/CR.
        for c in ['\u{1}', '\u{8}', '\u{b}', '\u{c}', '\u{1f}'] {
            assert!(
                matches!(
                    kind_of(parse_document(&format!("<a>x{c}y</a>"))),
                    XmlErrorKind::Malformed("control character forbidden by XML 1.0")
                ),
                "text U+{:04X} must be rejected",
                c as u32
            );
            assert!(
                matches!(
                    kind_of(parse_document(&format!("<a x=\"a{c}b\"/>"))),
                    XmlErrorKind::Malformed("control character forbidden by XML 1.0")
                ),
                "attribute U+{:04X} must be rejected",
                c as u32
            );
        }
    }

    #[test]
    fn tab_lf_cr_whitespace_controls_stay_accepted() {
        let root = parse_document("<a x=\"p\tq\">l1\nl2\r\nl3\tend</a>").expect("parse");
        assert_eq!(root.attr("x"), Some("p\tq"));
        assert_eq!(root.text(), "l1\nl2\r\nl3\tend");
    }

    #[test]
    fn control_char_error_position_is_accurate() {
        let doc = "<a>ok\u{0}</a>";
        let err = parse_document(doc).expect_err("must fail");
        assert_eq!(doc.as_bytes()[err.offset], 0);
        assert_eq!(err.line, 1);
        assert_eq!(err.col, 6);
    }

    #[test]
    fn garbage_before_root_rejected() {
        assert!(matches!(
            kind_of(parse_document("hello <a/>")),
            XmlErrorKind::Malformed("expected the document element")
        ));
    }

    #[test]
    fn depth_bound_enforced_iteratively() {
        // 1000 nested opens: must fail with DepthExceeded (bound 64), and
        // must not overflow the thread stack (parser is iterative).
        let mut doc = String::new();
        for _ in 0..1000 {
            doc.push_str("<d>");
        }
        assert_eq!(
            kind_of(parse_document(&doc)),
            XmlErrorKind::DepthExceeded(MAX_DEPTH)
        );
    }

    #[test]
    fn error_position_is_line_and_column_accurate() {
        // The unknown entity sits on line 3, after four bytes of indent.
        let doc = "<a>\n  <b>\n    &bad;\n  </b>\n</a>";
        let err = parse_document(doc).expect_err("must fail");
        assert_eq!(err.kind, XmlErrorKind::UnknownEntity("bad".to_owned()));
        assert_eq!(err.line, 3);
        assert_eq!(err.col, 5);
        assert_eq!(&doc[err.offset..err.offset + 5], "&bad;");
    }

    #[test]
    fn line_col_helper_clamps_and_counts() {
        assert_eq!(line_col("ab\ncd", 0), (1, 1));
        assert_eq!(line_col("ab\ncd", 3), (2, 1));
        assert_eq!(line_col("ab\ncd", 4), (2, 2));
        assert_eq!(line_col("ab\ncd", 999), (2, 3));
    }

    #[test]
    fn parse_is_deterministic_two_runs() {
        let doc = "<a x=\"1\"><b>t &amp; u</b><c/></a>";
        assert_eq!(parse_document(doc), parse_document(doc));
    }

    // ------------------------------------------------------------------
    // Tester round (2026-08-17): hostile/pathological input hardening.
    // ------------------------------------------------------------------

    #[test]
    fn ten_thousand_sibling_elements_parse_flat() {
        // Width stress (depth stays 2 — the depth bound is not in play):
        // 10k siblings must parse, count exactly, and preserve order.
        let mut doc = String::from("<root>");
        for i in 0..10_000 {
            doc.push_str(&format!("<e n=\"{i}\"/>"));
        }
        doc.push_str("</root>");
        let root = parse_document(&doc).expect("flat width must parse");
        let kids: Vec<&Element<'_>> = root.child_elements().collect();
        assert_eq!(kids.len(), 10_000);
        assert_eq!(kids[0].attr("n"), Some("0"));
        assert_eq!(kids[9_999].attr("n"), Some("9999"));
    }

    #[test]
    fn one_megabyte_attribute_value_roundtrips() {
        // 1 MB attribute value, no entities: must parse and come back
        // byte-identical (borrowed, no truncation).
        let big = "v".repeat(1_048_576);
        let doc = format!("<a x=\"{big}\"/>");
        let root = parse_document(&doc).expect("1 MB attribute must parse");
        let v = root.attr("x").expect("attr");
        assert_eq!(v.len(), 1_048_576);
        assert_eq!(v, big);
    }

    #[test]
    fn element_name_length_is_unbounded_100kb_name_accepted() {
        // PINNED CONTRACT: the name production has NO length limit (the
        // scan loop is unbounded), so a 100 KB element name is accepted
        // as long as the document is otherwise well-formed. Zero-copy
        // slices keep this O(n); acceptance is the actual behavior.
        let name = "n".repeat(100_000);
        let doc = format!("<{name}></{name}>");
        let root = parse_document(&doc).expect("100 KB name parses");
        assert_eq!(root.name.len(), 100_000);
        // Same for an attribute name.
        let doc2 = format!("<a {name}=\"1\"/>");
        let root2 = parse_document(&doc2).expect("100 KB attr name parses");
        assert_eq!(root2.attrs[0].name.len(), 100_000);
    }

    #[test]
    fn mixed_crlf_lf_line_and_column_reporting_stays_accurate() {
        // Line counting is \n-based, so CRLF and LF may mix freely; the
        // error on (logical) line 4 must be reported there. The \r at
        // the end of a CRLF line is invisible to the NEXT line's column.
        let doc = "<a>\r\n<b>\n<c>\r\n  &bad;\n</c>\n</b>\n</a>";
        let err = parse_document(doc).expect_err("unknown entity");
        assert_eq!(err.kind, XmlErrorKind::UnknownEntity("bad".to_owned()));
        assert_eq!(err.line, 4);
        assert_eq!(err.col, 3);
        assert_eq!(&doc[err.offset..err.offset + 5], "&bad;");
    }

    #[test]
    fn input_truncated_inside_entity_at_final_byte() {
        // The text run reaches EOF mid-entity: must be a typed Err
        // (never a panic or hang), positioned at the '&'.
        for doc in ["<a>&", "<a>&am", "<a>&amp"] {
            let err = parse_document(doc).expect_err("truncated entity");
            assert!(
                matches!(
                    err.kind,
                    XmlErrorKind::Malformed("unterminated entity reference")
                ),
                "{doc:?} -> {:?}",
                err.kind
            );
            assert_eq!(err.offset, 3, "{doc:?}");
        }
        // Entity COMPLETE at the very last byte, but element unclosed:
        // the entity decodes; the error is the unclosed element.
        assert!(matches!(
            kind_of(parse_document("<a>&amp;")),
            XmlErrorKind::UnexpectedEof("unclosed element")
        ));
    }

    #[test]
    fn comment_double_dash_variants() {
        // PINNED CONTRACT: comments terminate at the FIRST `-->`;
        // interior `--` (forbidden by strict XML 1.0) is accepted
        // because comment bodies are skipped uninspected and dropped.
        let root = parse_document("<a><!-- x -- y --></a>").expect("interior --");
        assert!(root.children.is_empty());
        // `<!----->` = `<!--` + `-` body + `-->` (greedy-first match).
        let root = parse_document("<a><!-----></a>").expect("dash body");
        assert!(root.children.is_empty());
        // `<!--->` never reaches a `-->`: typed EOF error.
        assert!(matches!(
            kind_of(parse_document("<a><!---></a>")),
            XmlErrorKind::UnexpectedEof("comment")
        ));
        // Empty comment `<!---->` is fine.
        let root = parse_document("<a><!----></a>").expect("empty comment");
        assert!(root.children.is_empty());
    }

    #[test]
    fn empty_and_whitespace_only_documents_rejected() {
        for doc in ["", "   \n\t\r\n  ", "\u{FEFF}", "\u{FEFF}  \n "] {
            assert!(
                matches!(
                    kind_of(parse_document(doc)),
                    XmlErrorKind::Malformed("expected the document element")
                ),
                "{doc:?} must report a missing document element"
            );
        }
    }

    #[test]
    fn second_consecutive_bom_is_content_error() {
        // Offset-0-only rule: the first BOM is skipped, the second is
        // U+FEFF character data before the root — a content error, not
        // a silently skipped second BOM.
        let err = parse_document("\u{FEFF}\u{FEFF}<a/>").expect_err("must fail");
        assert_eq!(
            err.kind,
            XmlErrorKind::Malformed("expected the document element")
        );
        assert_eq!(err.offset, 3, "error points at the second BOM's bytes");
        // Control: exactly one BOM parses.
        assert!(parse_document("\u{FEFF}<a/>").is_ok());
    }

    #[test]
    fn overlong_entity_name_rejected_via_length_bound() {
        // Names longer than MAX_ENTITY_LEN are cut off by the scan
        // bound and rejected. PINNED: the kind is the (slightly
        // misleading) "unterminated entity reference" even though a ';'
        // exists further out — behavior is a typed Err either way.
        assert!(matches!(
            kind_of(parse_document("<a>&notarealentity;</a>")),
            XmlErrorKind::Malformed("unterminated entity reference")
        ));
        // At the bound (8 chars) with a ';' present: unknown entity,
        // reported by name.
        assert_eq!(
            kind_of(parse_document("<a>&abcdefgh;</a>")),
            XmlErrorKind::UnknownEntity("abcdefgh".to_owned())
        );
    }
}
