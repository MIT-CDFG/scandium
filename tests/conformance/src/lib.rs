// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-conformance` — O-QA conformance harness, wave 1 (CP-0.1).
//!
//! Spec: plan AGENT-ASSIGNMENTS-full-build.md §2.1 row "Test harness v1";
//! ES §10.2 (appname.command.v1 envelope), §40.1 (error taxonomy),
//! §34.4 (generated property suites); determinism spine D-48.
//!
//! Wave-1 scope: load the committed `fixtures/adversarial/` corpus and
//! validate it against the frozen `contracts/` mirror. Wave-4 activation:
//! the engine suites run LIVE against `appname_core::Engine` — corpus
//! envelopes reach the engine as RAW BYTE SLICES of the committed corpus
//! files (binding directive WD-1: never a re-serialized parsed value; the
//! [`jsonspan`] scanner yields exact byte ranges, fidelity-checked). No
//! external dependencies (bootstrap rule): JSON is read by the strict
//! minimal parser in [`json`]; `appname-core` is an internal workspace
//! path dependency.

pub mod json {
    //! Strict minimal JSON reader for the committed corpus.
    //!
    //! Spec: RFC 8259 grammar subset sufficient for corpus files; used in
    //! place of an external crate per the CP-0.1 zero-dependency bootstrap
    //! rule (plan §2.1). Strictness is deliberate: duplicate keys, trailing
    //! input, control characters in strings, and non-finite numbers are
    //! errors — the corpus generator never emits them.

    use std::collections::BTreeMap;

    /// Parsed JSON value. Objects use `BTreeMap`: deterministic iteration
    /// order (D-48 discipline) and duplicate-key rejection at insert time.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Value {
        Null,
        Bool(bool),
        Num(f64),
        Str(String),
        Arr(Vec<Value>),
        Obj(BTreeMap<String, Value>),
    }

    impl Value {
        pub fn get(&self, key: &str) -> Option<&Value> {
            match self {
                Value::Obj(m) => m.get(key),
                _ => None,
            }
        }
        pub fn as_str(&self) -> Option<&str> {
            match self {
                Value::Str(s) => Some(s),
                _ => None,
            }
        }
        pub fn as_arr(&self) -> Option<&[Value]> {
            match self {
                Value::Arr(a) => Some(a),
                _ => None,
            }
        }
        pub fn as_obj(&self) -> Option<&BTreeMap<String, Value>> {
            match self {
                Value::Obj(m) => Some(m),
                _ => None,
            }
        }
        pub fn as_bool(&self) -> Option<bool> {
            match self {
                Value::Bool(b) => Some(*b),
                _ => None,
            }
        }
        pub fn as_i64(&self) -> Option<i64> {
            match self {
                Value::Num(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Some(*n as i64),
                _ => None,
            }
        }
        pub fn is_null(&self) -> bool {
            matches!(self, Value::Null)
        }
    }

    const MAX_DEPTH: usize = 128;

    struct Parser<'a> {
        b: &'a [u8],
        pos: usize,
    }

    /// Parse a complete JSON document; trailing non-whitespace is an error.
    pub fn parse(text: &str) -> Result<Value, String> {
        let mut p = Parser {
            b: text.as_bytes(),
            pos: 0,
        };
        p.ws();
        let v = p.value(0)?;
        p.ws();
        if p.pos != p.b.len() {
            return Err(p.err("trailing input after JSON document"));
        }
        Ok(v)
    }

    impl<'a> Parser<'a> {
        fn err(&self, msg: &str) -> String {
            format!("json error at byte {}: {}", self.pos, msg)
        }
        fn ws(&mut self) {
            while let Some(c) = self.b.get(self.pos) {
                match c {
                    b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                    _ => break,
                }
            }
        }
        fn peek(&self) -> Option<u8> {
            self.b.get(self.pos).copied()
        }
        fn eat(&mut self, c: u8) -> Result<(), String> {
            if self.peek() == Some(c) {
                self.pos += 1;
                Ok(())
            } else {
                Err(self.err(&format!("expected '{}'", c as char)))
            }
        }
        fn lit(&mut self, s: &str, v: Value) -> Result<Value, String> {
            if self.b[self.pos..].starts_with(s.as_bytes()) {
                self.pos += s.len();
                Ok(v)
            } else {
                Err(self.err(&format!("expected literal '{s}'")))
            }
        }

        fn value(&mut self, depth: usize) -> Result<Value, String> {
            if depth > MAX_DEPTH {
                return Err(self.err("nesting too deep"));
            }
            match self.peek() {
                Some(b'{') => self.object(depth),
                Some(b'[') => self.array(depth),
                Some(b'"') => Ok(Value::Str(self.string()?)),
                Some(b't') => self.lit("true", Value::Bool(true)),
                Some(b'f') => self.lit("false", Value::Bool(false)),
                Some(b'n') => self.lit("null", Value::Null),
                Some(c) if c == b'-' || c.is_ascii_digit() => self.number(),
                _ => Err(self.err("expected a JSON value")),
            }
        }

        fn object(&mut self, depth: usize) -> Result<Value, String> {
            self.eat(b'{')?;
            let mut m = BTreeMap::new();
            self.ws();
            if self.peek() == Some(b'}') {
                self.pos += 1;
                return Ok(Value::Obj(m));
            }
            loop {
                self.ws();
                let k = self.string()?;
                self.ws();
                self.eat(b':')?;
                self.ws();
                let v = self.value(depth + 1)?;
                if m.insert(k.clone(), v).is_some() {
                    return Err(self.err(&format!("duplicate key '{k}'")));
                }
                self.ws();
                match self.peek() {
                    Some(b',') => self.pos += 1,
                    Some(b'}') => {
                        self.pos += 1;
                        return Ok(Value::Obj(m));
                    }
                    _ => return Err(self.err("expected ',' or '}'")),
                }
            }
        }

        fn array(&mut self, depth: usize) -> Result<Value, String> {
            self.eat(b'[')?;
            let mut a = Vec::new();
            self.ws();
            if self.peek() == Some(b']') {
                self.pos += 1;
                return Ok(Value::Arr(a));
            }
            loop {
                self.ws();
                a.push(self.value(depth + 1)?);
                self.ws();
                match self.peek() {
                    Some(b',') => self.pos += 1,
                    Some(b']') => {
                        self.pos += 1;
                        return Ok(Value::Arr(a));
                    }
                    _ => return Err(self.err("expected ',' or ']'")),
                }
            }
        }

        fn string(&mut self) -> Result<String, String> {
            self.eat(b'"')?;
            let mut s = String::new();
            loop {
                let c = self.peek().ok_or_else(|| self.err("unterminated string"))?;
                self.pos += 1;
                match c {
                    b'"' => return Ok(s),
                    b'\\' => {
                        let e = self.peek().ok_or_else(|| self.err("bad escape"))?;
                        self.pos += 1;
                        match e {
                            b'"' => s.push('"'),
                            b'\\' => s.push('\\'),
                            b'/' => s.push('/'),
                            b'b' => s.push('\u{0008}'),
                            b'f' => s.push('\u{000C}'),
                            b'n' => s.push('\n'),
                            b'r' => s.push('\r'),
                            b't' => s.push('\t'),
                            b'u' => {
                                let hi = self.hex4()?;
                                let cp = if (0xD800..0xDC00).contains(&hi) {
                                    // surrogate pair
                                    self.eat(b'\\')?;
                                    self.eat(b'u')?;
                                    let lo = self.hex4()?;
                                    if !(0xDC00..0xE000).contains(&lo) {
                                        return Err(self.err("bad low surrogate"));
                                    }
                                    0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                                } else if (0xDC00..0xE000).contains(&hi) {
                                    return Err(self.err("lone surrogate"));
                                } else {
                                    hi
                                };
                                s.push(
                                    char::from_u32(cp).ok_or_else(|| self.err("bad codepoint"))?,
                                );
                            }
                            _ => return Err(self.err("unknown escape")),
                        }
                    }
                    0x00..=0x1F => return Err(self.err("raw control char in string")),
                    _ => {
                        // re-take the full UTF-8 scalar starting at c
                        self.pos -= 1;
                        let rest = &self.b[self.pos..];
                        let text =
                            std::str::from_utf8(rest).map_err(|_| self.err("invalid utf-8"))?;
                        let ch = text.chars().next().unwrap();
                        s.push(ch);
                        self.pos += ch.len_utf8();
                    }
                }
            }
        }

        fn hex4(&mut self) -> Result<u32, String> {
            let mut v = 0u32;
            for _ in 0..4 {
                let c = self.peek().ok_or_else(|| self.err("bad \\u"))?;
                self.pos += 1;
                v = v * 16
                    + (c as char)
                        .to_digit(16)
                        .ok_or_else(|| self.err("bad hex digit"))?;
            }
            Ok(v)
        }

        fn number(&mut self) -> Result<Value, String> {
            let start = self.pos;
            if self.peek() == Some(b'-') {
                self.pos += 1;
            }
            // int part
            match self.peek() {
                Some(b'0') => self.pos += 1,
                Some(c) if c.is_ascii_digit() => {
                    while matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                        self.pos += 1;
                    }
                }
                _ => return Err(self.err("bad number")),
            }
            if self.peek() == Some(b'.') {
                self.pos += 1;
                if !matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                    return Err(self.err("bad fraction"));
                }
                while matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                    self.pos += 1;
                }
            }
            if matches!(self.peek(), Some(b'e') | Some(b'E')) {
                self.pos += 1;
                if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                    self.pos += 1;
                }
                if !matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                    return Err(self.err("bad exponent"));
                }
                while matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                    self.pos += 1;
                }
            }
            let raw = std::str::from_utf8(&self.b[start..self.pos]).unwrap();
            let n: f64 = raw.parse().map_err(|_| self.err("number out of range"))?;
            if !n.is_finite() {
                return Err(self.err("non-finite number"));
            }
            Ok(Value::Num(n))
        }
    }

    #[cfg(test)]
    mod tests {
        //! Spec: parser unit checks (RFC 8259 subset; strictness contract).
        use super::{parse, Value};

        #[test]
        fn parses_scalars_arrays_objects() {
            let v = parse(r#"{"a":[1,2.5,-3e2],"b":"xé\n","c":true,"d":null}"#).unwrap();
            assert_eq!(v.get("a").unwrap().as_arr().unwrap().len(), 3);
            assert_eq!(v.get("b").unwrap().as_str().unwrap(), "x\u{e9}\n");
            assert_eq!(v.get("c").unwrap().as_bool(), Some(true));
            assert!(v.get("d").unwrap().is_null());
            assert_eq!(v.get("a").unwrap().as_arr().unwrap()[2], Value::Num(-300.0));
        }

        #[test]
        fn surrogate_pairs_decode() {
            let v = parse(r#""😀""#).unwrap();
            assert_eq!(v.as_str().unwrap(), "\u{1F600}");
        }

        #[test]
        fn rejects_trailing_and_duplicates_and_bad_escapes() {
            assert!(parse("{} x").is_err());
            assert!(parse(r#"{"k":1,"k":2}"#).is_err());
            assert!(parse(r#""\q""#).is_err());
            assert!(parse("01").is_err());
            assert!(parse("[1,]").is_err());
            assert!(parse("nul").is_err());
        }

        #[test]
        fn object_iteration_is_sorted() {
            let v = parse(r#"{"z":1,"a":2,"m":3}"#).unwrap();
            let keys: Vec<_> = v.as_obj().unwrap().keys().cloned().collect();
            assert_eq!(keys, ["a", "m", "z"]);
        }
    }
}

pub mod jsonspan {
    //! Byte-span scanner for the committed corpus files — the WD-1
    //! instrument. Walks the same strict JSON grammar as [`crate::json`]
    //! but yields, for every node, its exact byte range in the source
    //! text, so an envelope can be handed to the engine as the VERBATIM
    //! slice of the corpus file (never re-serialized: `1.0` stays `1.0`).
    //!
    //! Spec: binding directive WD-1 (reports/O-QA-log.md); RFC 8259
    //! grammar subset. Keys are captured raw and must not contain escapes
    //! (the generator never emits any); violations panic loudly.

    use std::collections::BTreeMap;

    /// One JSON node: its byte range plus keyed/indexed children.
    pub struct Node {
        pub start: usize,
        pub end: usize,
        pub children: Children,
    }

    pub enum Children {
        Obj(BTreeMap<String, Node>),
        Arr(Vec<Node>),
        Leaf,
    }

    impl Node {
        pub fn get(&self, key: &str) -> Option<&Node> {
            match &self.children {
                Children::Obj(m) => m.get(key),
                _ => None,
            }
        }
        pub fn idx(&self, i: usize) -> Option<&Node> {
            match &self.children {
                Children::Arr(a) => a.get(i),
                _ => None,
            }
        }
        pub fn raw<'a>(&self, text: &'a str) -> &'a str {
            &text[self.start..self.end]
        }
    }

    pub fn parse(text: &str) -> Node {
        let b = text.as_bytes();
        let mut pos = 0usize;
        skip_ws(b, &mut pos);
        let node = value(b, &mut pos);
        skip_ws(b, &mut pos);
        assert_eq!(pos, b.len(), "jsonspan: trailing input");
        node
    }

    fn skip_ws(b: &[u8], pos: &mut usize) {
        while matches!(b.get(*pos), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            *pos += 1;
        }
    }

    fn value(b: &[u8], pos: &mut usize) -> Node {
        let start = *pos;
        let children = match b.get(*pos) {
            Some(b'{') => {
                *pos += 1;
                let mut m = BTreeMap::new();
                skip_ws(b, pos);
                if b.get(*pos) == Some(&b'}') {
                    *pos += 1;
                } else {
                    loop {
                        skip_ws(b, pos);
                        let key = raw_string(b, pos);
                        skip_ws(b, pos);
                        assert_eq!(b.get(*pos), Some(&b':'), "jsonspan ':'");
                        *pos += 1;
                        skip_ws(b, pos);
                        let child = value(b, pos);
                        assert!(!key.contains('\\'), "jsonspan: escaped key unsupported");
                        m.insert(key, child);
                        skip_ws(b, pos);
                        match b.get(*pos) {
                            Some(b',') => *pos += 1,
                            Some(b'}') => {
                                *pos += 1;
                                break;
                            }
                            other => panic!("jsonspan obj: {other:?}"),
                        }
                    }
                }
                Children::Obj(m)
            }
            Some(b'[') => {
                *pos += 1;
                let mut a = Vec::new();
                skip_ws(b, pos);
                if b.get(*pos) == Some(&b']') {
                    *pos += 1;
                } else {
                    loop {
                        skip_ws(b, pos);
                        a.push(value(b, pos));
                        skip_ws(b, pos);
                        match b.get(*pos) {
                            Some(b',') => *pos += 1,
                            Some(b']') => {
                                *pos += 1;
                                break;
                            }
                            other => panic!("jsonspan arr: {other:?}"),
                        }
                    }
                }
                Children::Arr(a)
            }
            Some(b'"') => {
                raw_string(b, pos);
                Children::Leaf
            }
            Some(b't') => {
                *pos += 4;
                Children::Leaf
            }
            Some(b'f') => {
                *pos += 5;
                Children::Leaf
            }
            Some(b'n') => {
                *pos += 4;
                Children::Leaf
            }
            Some(c) if *c == b'-' || c.is_ascii_digit() => {
                while matches!(
                    b.get(*pos),
                    Some(c) if c.is_ascii_digit()
                        || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E')
                ) {
                    *pos += 1;
                }
                Children::Leaf
            }
            other => panic!("jsonspan value: {other:?}"),
        };
        Node {
            start,
            end: *pos,
            children,
        }
    }

    /// Scan a JSON string, returning its DECODED content only when it has
    /// no escapes (corpus keys never do); the cursor ends past the close
    /// quote either way.
    fn raw_string(b: &[u8], pos: &mut usize) -> String {
        assert_eq!(b.get(*pos), Some(&b'"'), "jsonspan '\"'");
        *pos += 1;
        let content_start = *pos;
        let mut escaped = false;
        loop {
            match b.get(*pos) {
                Some(b'\\') => {
                    escaped = true;
                    *pos += 2;
                }
                Some(b'"') => break,
                Some(_) => *pos += 1,
                None => panic!("jsonspan: unterminated string"),
            }
        }
        let content = if escaped {
            String::from("\\") // marker: contains escapes; keys reject it
        } else {
            String::from_utf8(b[content_start..*pos].to_vec()).expect("corpus is UTF-8")
        };
        *pos += 1;
        content
    }

    #[cfg(test)]
    mod tests {
        //! Spec: WD-1 span fidelity — the slice is the verbatim source.
        use super::parse;

        #[test]
        fn spans_are_verbatim_slices() {
            let text = r#"{"cases":[{"envelope":{"a":1.0,"b":[1,2]},"x":null}]}"#;
            let root = parse(text);
            let env = root
                .get("cases")
                .and_then(|c| c.idx(0))
                .and_then(|c| c.get("envelope"))
                .unwrap();
            assert_eq!(env.raw(text), r#"{"a":1.0,"b":[1,2]}"#);
            // the float survives byte-for-byte — the WD-1 point
            assert!(env.raw(text).contains("1.0"));
        }
    }
}

pub mod harness {
    //! Live-engine case runner (wave-4 activation).
    //!
    //! Spec: plan §2.1 "Test harness v1"; WD-1 (raw bytes); pinned
    //! validation order (reports/O-CORE-log.md, OQ-3 closed); wave-2
    //! binding note: ported/envelope.json holds only under
    //! primed-sequential semantics.

    use crate::json::{parse, Value};
    use crate::jsonspan;
    use appname_core::Engine;

    /// Engine outcome normalized for corpus assertion.
    pub struct Outcome {
        pub ok: bool,
        pub code: Option<String>,
        pub message: String,
    }

    /// Apply one RAW envelope slice (WD-1) through the single entry.
    pub fn apply_raw(engine: &mut Engine, raw: &str) -> Outcome {
        match engine.apply_bytes(raw.as_bytes()) {
            Ok(_) => Outcome {
                ok: true,
                code: None,
                message: String::new(),
            },
            Err(e) => Outcome {
                ok: false,
                code: Some(e.code.as_str().to_string()),
                message: e.message.clone(),
            },
        }
    }

    /// Extract the raw slice for `cases[i].<key>` (or a sequence element)
    /// and fidelity-check it against the parsed corpus value.
    pub fn case_raw<'a>(
        text: &'a str,
        span: &jsonspan::Node,
        doc: &Value,
        case_idx: usize,
        key: &str,
        seq_idx: Option<usize>,
    ) -> &'a str {
        let mut node = span
            .get("cases")
            .and_then(|c| c.idx(case_idx))
            .and_then(|c| c.get(key))
            .unwrap_or_else(|| panic!("no span for cases[{case_idx}].{key}"));
        let mut expect = doc
            .get("cases")
            .and_then(Value::as_arr)
            .and_then(|a| a.get(case_idx))
            .and_then(|c| c.get(key))
            .unwrap_or_else(|| panic!("no value at cases[{case_idx}].{key}"));
        if let Some(j) = seq_idx {
            node = node.idx(j).unwrap_or_else(|| panic!("no span seq[{j}]"));
            expect = expect
                .as_arr()
                .and_then(|a| a.get(j))
                .unwrap_or_else(|| panic!("no value seq[{j}]"));
        }
        let raw = node.raw(text);
        // WD-1 fidelity gate: the slice parses to exactly the corpus value.
        let reparsed = parse(raw).unwrap_or_else(|e| panic!("raw slice does not parse: {e}"));
        assert!(
            &reparsed == expect,
            "span fidelity broke at cases[{case_idx}].{key}"
        );
        raw
    }

    /// The expectation record for a case under a given assembly:
    /// `cp03Assembled` overlays `cp01Expect` on assembled engines where
    /// present; everything else carries `cp01Expect` forward (the corpus
    /// manifest `cp03Overlay.rule`).
    pub fn expected_for(case: &Value, assembled: bool) -> Option<&Value> {
        if assembled {
            if let Some(over) = case.get("cp03Assembled").filter(|v| !v.is_null()) {
                return Some(over);
            }
            // kind-file overlay: the caller resolves file-level cp03
            // (kinds/<kind>.json `cp03.assembled`) before calling; the
            // per-case field wins when both exist.
        }
        case.get("cp01Expect").filter(|v| !v.is_null())
    }

    /// Assert one engine outcome against the case's expectation under the
    /// given assembly. Exit classes are a CLI concern; at engine level
    /// the outcome + code carry the contract.
    pub fn check_case_asm(
        case: &Value,
        got: &Outcome,
        assembled: bool,
        file_overlay: Option<&Value>,
    ) -> Result<(), String> {
        let cid = case
            .get("caseId")
            .and_then(Value::as_str)
            .unwrap_or("<no caseId>");
        let per_case = case.get("cp03Assembled").filter(|v| !v.is_null());
        let base = case.get("cp01Expect").filter(|v| !v.is_null());
        let cp01 = if assembled {
            per_case.or(file_overlay).or(base)
        } else {
            base
        }
        .ok_or_else(|| format!("{cid}: no expectation on runnable case"))?;
        let outcome = cp01.get("outcome").and_then(Value::as_str).unwrap();
        if (outcome == "accept") != got.ok {
            return Err(format!(
                "{cid}: wanted {outcome}, engine said ok={} code={:?} \
                 message={:?}",
                got.ok, got.code, got.message
            ));
        }
        if !got.ok {
            let code = got.code.as_deref().unwrap_or("");
            if !code.starts_with("error.") || got.message.is_empty() {
                return Err(format!(
                    "{cid}: rejection breaks the machine contract \
                     (code={code:?}, message empty={})",
                    got.message.is_empty()
                ));
            }
            if let Some(want) = cp01.get("code").and_then(Value::as_str) {
                if code != want {
                    return Err(format!(
                        "{cid}: wanted code {want}, got {code} ({})",
                        got.message
                    ));
                }
            }
        }
        Ok(())
    }

    /// Bare-core assertion (the wave-4 surface, unchanged behavior).
    pub fn check_case(case: &Value, got: &Outcome) -> Result<(), String> {
        check_case_asm(case, got, false, None)
    }

    pub fn is_runnable(case: &Value) -> bool {
        case.get("cp01Runnable")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

pub mod goldens {
    //! doc.new golden matrix — QA-owned, ENGINE-PRODUCED (wave 4).
    //!
    //! Spec: plan §2.1 "Test harness v1" (golden fixtures, deterministic,
    //! seeded/fixed inputs) + §0.6 rule 6 (goldens change only via O-QA
    //! review); ES §7.6 (deterministic package writer); goldens/README.md
    //! rules. Inputs are FIXED envelopes recorded in each golden's .meta;
    //! the regen bin (`regen_doc_new_goldens`) writes files, the verify
    //! test regenerates in memory and byte-compares — rule 7: expected
    //! bytes only ever come from the engine, never a hand.

    use appname_core::{export_png_bytes, save_package_bytes, Engine};

    /// One golden combo: fixed doc.new inputs.
    pub struct Combo {
        pub name: String,
        pub envelope: String,
    }

    /// The fixed matrix: modes x sizes x backgrounds (depth u8 — the only
    /// depth live at CP-0.1; cmyk_a refuses at stage 6d and is exercised
    /// by the adversarial corpus instead). Envelope ids follow the
    /// 601d-prefix positional scheme over this exact order.
    pub fn doc_new_matrix() -> Vec<Combo> {
        let modes = ["rgba", "gray_a", "lab_a"];
        let sizes = [(1u32, 1u32), (96, 64)];
        let backgrounds = ["transparent", "white", "black"];
        let mut out = Vec::new();
        let mut ordinal = 0u64;
        for mode in modes {
            for (w, h) in sizes {
                for bg in backgrounds {
                    ordinal += 1;
                    let id = format!("601d0000-0000-4000-8000-{ordinal:012}");
                    out.push(Combo {
                        name: format!("doc_new-{mode}-{w}x{h}-{bg}"),
                        envelope: format!(
                            r#"{{"schema":"appname.command.v1","id":"{id}","kind":"doc.new","params":{{"width":{w},"height":{h},"mode":"{mode}","depth":"u8","background":"{bg}"}}}}"#
                        ),
                    });
                }
            }
        }
        out
    }

    /// The selftest reference inputs, mirrored VERBATIM from the CLI
    /// selftest's fixed script (crates/appname-cli/src/selftest.rs):
    /// since the CP-0.3 stroke wiring the script is doc.new + one brush
    /// stroke over the fixed samples asset. The package sha256/size and
    /// state_hash are recorded in reports/O-CORE-log.md (stroke-wiring
    /// entry); the pre-stroke reference (33818115…, 527133 B) is retired
    /// with this revision.
    pub const SELFTEST_DOC_NEW: &str = r#"{"schema":"appname.command.v1","id":"5e1f7e57-0001-4000-8000-000000000001","kind":"doc.new","params":{"width":320,"height":200,"background":"black"}}"#;
    pub const SELFTEST_REFERENCE_SHA256: &str =
        "1edbe91516a30728b3c2c0598f5084d55a3a3aa6640dfa57be1cedc2aadefa1a";
    pub const SELFTEST_REFERENCE_BYTES: usize = 527851;
    pub const SELFTEST_STATE_HASH_PREFIX: &str = "71ef71521484";

    /// The selftest fixed samples (mirrored: 3 points, pressure 1).
    pub fn selftest_fixed_samples() -> Vec<u8> {
        use appname_tools::brush::{encode_samples, InputSample};
        let pts: [(f64, f64, f64); 3] =
            [(40.0, 30.0, 0.0), (160.0, 90.0, 0.05), (280.0, 150.0, 0.10)];
        let samples: Vec<InputSample> = pts
            .iter()
            .map(|(x, y, t)| InputSample {
                x: *x,
                y: *y,
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                velocity: 0.0,
                t: *t,
            })
            .collect();
        encode_samples(&samples).expect("fixed samples encode")
    }

    /// Reproduce the selftest reference package mechanically (assembled
    /// engine; asset first, then the two script envelopes — the one_run
    /// order), returning (package bytes, state_hash hex).
    pub fn selftest_reference_package() -> (Vec<u8>, String) {
        let mut engine = crate::assembly_point::assembled_engine();
        let samples_id = engine.add_samples_asset(selftest_fixed_samples());
        let receipt = engine
            .apply_bytes(SELFTEST_DOC_NEW.as_bytes())
            .expect("selftest doc.new");
        let layer = receipt.created_ids.last().copied().expect("bg layer");
        let brush = format!(
            r#"{{"schema":"appname.command.v1","id":"5e1f7e57-0001-4000-8000-000000000002","kind":"brush.stroke","target":{{"layerId":"{layer}"}},"params":{{"brushPreset":"{preset}","color":{{"model":"rgb","comps":[1,0.25,0,0]}},"seed":7,"opacity":0.9}},"samples":"{samples_id}"}}"#,
            preset = crate::assembly_point::preset_hard_round_16_id(),
        );
        engine
            .apply_bytes(brush.as_bytes())
            .expect("selftest brush");
        let pkg = save_package_bytes(&engine).expect("selftest save");
        let hash = format!("{}", engine.state_hash());
        (pkg, hash)
    }

    /// Produce (package bytes, export PNG bytes if available) for one
    /// fixed envelope through the public engine entry.
    ///
    /// PNG export of non-rgba modes answers `error.command.not_implemented`
    /// at CP-0.1 (engine boundary: the §5/§6 conversion pipeline arrives
    /// at CP-0.5) — recorded per golden as a DEFERRED export, never
    /// papered over: those combos carry a package golden only, and their
    /// .meta names the engine's own deferral message.
    pub fn produce(envelope: &str) -> (Vec<u8>, Result<Vec<u8>, String>) {
        let mut engine = Engine::new();
        engine
            .apply_bytes(envelope.as_bytes())
            .unwrap_or_else(|e| panic!("golden envelope rejected: {e}"));
        let package =
            save_package_bytes(&engine).unwrap_or_else(|e| panic!("golden package save: {e}"));
        let export = match export_png_bytes(&engine) {
            Ok(png) => Ok(png),
            Err(e) if e.code.as_str() == "error.command.not_implemented" => {
                Err(format!("deferred: {}", e.message))
            }
            Err(e) => panic!("golden export: {e}"),
        };
        (package, export)
    }
}

#[allow(clippy::excessive_precision)] // oracle constants transcribed to 9
                                      // decimals from the O-QA scalar formula evaluation (rule-7 fidelity pins;
                                      // same disposition as the O-ALGO ported-constant-table ruling)
pub mod blend_goldens {
    //! Compositor blend-mode goldens vs HAND-COMPUTED oracles (CP-0.3
    //! slice A, plan §2.2 row "Workflow gates"/"Golden expansion" seed).
    //!
    //! Spec: ES §4.4 (compositing algebra + the NORMATIVE worked example,
    //! which names itself a unit test of the golden suite §34.3), §4.5
    //! (blend catalog; blend set v1 = 8 modes at CP-0.3), §4.8 (working
    //! tiles); plan §0.6 rule 6 (goldens change only via O-QA review).
    //!
    //! The ORACLE side is O-QA's own derivation: [`PROBES`] carries
    //! expected (Co, αo) values computed independently from the §4.4/§4.5
    //! formula text (scalar calculation, steps recorded in each golden's
    //! .meta and in reports/O-QA-log.md) — deliberately NOT derived from
    //! appname-render's code or tests. The BYTE side is produced through
    //! appname-render's public API and frozen under goldens/blend/.
    //!
    //! Formula (per pixel, one channel; §4.4):
    //!   αs = sa·opacity;  Cs = premult_src/sa (sa>0 else 0);
    //!   Cb = premult_dst/αb (αb>0 else 0);
    //!   Co = αs·(1−αb)·Cs + αs·αb·B(Cb,Cs) + (1−αs)·αb·Cb
    //!   αo = αs + αb·(1−αs)

    use appname_core::{BlendMode, IRect};
    use appname_render::{composite_document, composite_tile, TileRgba};

    /// Golden window: 64x64 samples of the composited 256x256 tile at
    /// stride 4 — covers the full ramp domain compactly (64 KiB/golden).
    pub const WIN: usize = 64;
    pub const STRIDE: usize = 4;

    /// Deterministic input tiles, defined in closed form (recorded
    /// verbatim in each .meta). R=G=B by construction so oracle probes
    /// are single-channel.
    #[derive(Clone, Copy, PartialEq, Debug)]
    pub enum TileDef {
        /// Cs = x/255, sa = y/255 (stored premultiplied Cs·sa).
        SrcRamp,
        /// Cb = y/255, αb = x/255 (crossed against SrcRamp).
        BackdropRamp,
        /// HDR: Cs = 4x/255 (linear f32, up to 4.0), sa = y/255.
        HdrSrc,
        /// HDR backdrop: Cb = 2y/255, αb = x/255.
        HdrBackdrop,
        /// All scalars zero (a=0 everywhere; un-premultiply-guard probe).
        Zero,
        /// §4.4 worked-example source: stored (0.30, 0.50) uniform.
        SpecSrc,
        /// §4.4 worked-example backdrop: stored (0.40, 0.80) uniform.
        SpecBackdrop,
        /// CP-0.5 chromatic source (non-separable modes need chroma —
        /// gray is degenerate for Sat): R=x/255, G=y/255, B=0.25, sa=0.8.
        ChromaSrc,
        /// Chromatic backdrop: R=0.3, G=y/255, B=x/255, ab=0.9.
        ChromaBackdrop,
    }

    pub fn build_tile(def: TileDef) -> TileRgba {
        match def {
            TileDef::Zero => TileRgba::transparent(),
            TileDef::SpecSrc => TileRgba::splat([0.30, 0.30, 0.30, 0.50]),
            TileDef::SpecBackdrop => TileRgba::splat([0.40, 0.40, 0.40, 0.80]),
            TileDef::ChromaSrc | TileDef::ChromaBackdrop => {
                let mut t = TileRgba::transparent();
                for y in 0..256usize {
                    for x in 0..256usize {
                        let (rgb, a): ([f32; 3], f32) = match def {
                            TileDef::ChromaSrc => ([x as f32 / 255.0, y as f32 / 255.0, 0.25], 0.8),
                            TileDef::ChromaBackdrop => {
                                ([0.3, y as f32 / 255.0, x as f32 / 255.0], 0.9)
                            }
                            _ => unreachable!(),
                        };
                        t.set(x, y, [rgb[0] * a, rgb[1] * a, rgb[2] * a, a]);
                    }
                }
                t
            }
            _ => {
                let mut t = TileRgba::transparent();
                for y in 0..256usize {
                    for x in 0..256usize {
                        let (c, a) = match def {
                            TileDef::SrcRamp => (x as f32 / 255.0, y as f32 / 255.0),
                            TileDef::BackdropRamp => (y as f32 / 255.0, x as f32 / 255.0),
                            TileDef::HdrSrc => (4.0 * x as f32 / 255.0, y as f32 / 255.0),
                            TileDef::HdrBackdrop => (2.0 * y as f32 / 255.0, x as f32 / 255.0),
                            _ => unreachable!(),
                        };
                        let p = c * a; // stored premultiplied
                        t.set(x, y, [p, p, p, a]);
                    }
                }
                t
            }
        }
    }

    /// One golden case: a layer stack (bottom → top) over a backdrop.
    pub struct Case {
        pub name: &'static str,
        pub backdrop: TileDef,
        pub layers: &'static [(BlendMode, f32, TileDef)],
    }

    /// The fixed matrix (18 tile cases; the document-walk case is
    /// [`doc_walk_window`]). Order is the file order — stable.
    pub fn cases() -> Vec<Case> {
        use BlendMode as M;
        use TileDef as T;
        vec![
            // the §4.4 worked example, spec-mandated in the golden suite
            Case {
                name: "spec-worked-multiply",
                backdrop: T::SpecBackdrop,
                layers: &[(M::Multiply, 1.0, T::SpecSrc)],
            },
            Case {
                name: "spec-worked-normal",
                backdrop: T::SpecBackdrop,
                layers: &[(M::Normal, 1.0, T::SpecSrc)],
            },
            // blend set v1, one ramp case per mode
            Case {
                name: "mode-normal",
                backdrop: T::BackdropRamp,
                layers: &[(M::Normal, 1.0, T::SrcRamp)],
            },
            Case {
                name: "mode-darken",
                backdrop: T::BackdropRamp,
                layers: &[(M::Darken, 1.0, T::SrcRamp)],
            },
            Case {
                name: "mode-multiply",
                backdrop: T::BackdropRamp,
                layers: &[(M::Multiply, 1.0, T::SrcRamp)],
            },
            Case {
                name: "mode-lighten",
                backdrop: T::BackdropRamp,
                layers: &[(M::Lighten, 1.0, T::SrcRamp)],
            },
            Case {
                name: "mode-screen",
                backdrop: T::BackdropRamp,
                layers: &[(M::Screen, 1.0, T::SrcRamp)],
            },
            Case {
                name: "mode-linear_dodge",
                backdrop: T::BackdropRamp,
                layers: &[(M::LinearDodge, 1.0, T::SrcRamp)],
            },
            Case {
                name: "mode-overlay",
                backdrop: T::BackdropRamp,
                layers: &[(M::Overlay, 1.0, T::SrcRamp)],
            },
            Case {
                name: "mode-difference",
                backdrop: T::BackdropRamp,
                layers: &[(M::Difference, 1.0, T::SrcRamp)],
            },
            // opacity scaling accumulated across a two-composite stack
            Case {
                name: "opacity-stack-normal",
                backdrop: T::BackdropRamp,
                layers: &[(M::Normal, 0.5, T::SrcRamp), (M::Normal, 0.25, T::SrcRamp)],
            },
            // a = 0 everywhere: un-premultiply guard must yield zeros
            Case {
                name: "alpha-zero",
                backdrop: T::Zero,
                layers: &[(M::Normal, 1.0, T::Zero)],
            },
            // HDR f32 (values > 1 are legal working-space content)
            Case {
                name: "hdr-normal",
                backdrop: T::HdrBackdrop,
                layers: &[(M::Normal, 1.0, T::HdrSrc)],
            },
            Case {
                name: "hdr-multiply",
                backdrop: T::HdrBackdrop,
                layers: &[(M::Multiply, 1.0, T::HdrSrc)],
            },
            Case {
                name: "hdr-screen",
                backdrop: T::HdrBackdrop,
                layers: &[(M::Screen, 1.0, T::HdrSrc)],
            },
            Case {
                name: "hdr-linear_dodge",
                backdrop: T::HdrBackdrop,
                layers: &[(M::LinearDodge, 1.0, T::HdrSrc)],
            },
            // mixed-mode 4-layer stack with an opacity ladder
            Case {
                name: "stack-mixed-4",
                backdrop: T::BackdropRamp,
                layers: &[
                    (M::Multiply, 1.0, T::SrcRamp),
                    (M::Screen, 0.75, T::SrcRamp),
                    (M::Overlay, 0.5, T::SrcRamp),
                    (M::Difference, 0.25, T::SrcRamp),
                ],
            },
            // ---- CP-0.5 render-companion extension: the 19 remaining
            // modes (chromatic tiles; Appendix A formulas; oracles in
            // PROBES_RGB). Dissolve rides the documented zero-ctx door
            // and its own membership property in the verify suite.
            Case {
                name: "mode2-color_burn",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::ColorBurn, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-linear_burn",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::LinearBurn, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-darker_color",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::DarkerColor, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-color_dodge",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::ColorDodge, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-lighter_color",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::LighterColor, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-soft_light",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::SoftLight, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-hard_light",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::HardLight, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-vivid_light",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::VividLight, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-linear_light",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::LinearLight, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-pin_light",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::PinLight, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-hard_mix",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::HardMix, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-exclusion",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::Exclusion, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-subtract",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::Subtract, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-divide",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::Divide, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-hue",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::Hue, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-saturation",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::Saturation, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-color",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::Color, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-luminosity",
                backdrop: T::ChromaBackdrop,
                layers: &[(M::Luminosity, 1.0, T::ChromaSrc)],
            },
            Case {
                name: "mode2-dissolve",
                backdrop: T::BackdropRamp,
                layers: &[(M::Dissolve, 1.0, T::SrcRamp)],
            },
        ]
    }

    /// Composite one case through the PUBLIC tile API and extract the
    /// golden window as little-endian f32 bytes (row-major RGBA).
    pub fn produce_window(case: &Case) -> Vec<u8> {
        let mut dst = build_tile(case.backdrop);
        for (mode, opacity, def) in case.layers {
            let src = build_tile(*def);
            composite_tile(&mut dst, &src, *mode, *opacity)
                .unwrap_or_else(|e| panic!("{}: {e:?}", case.name));
        }
        window_bytes(|x, y| dst.get(x, y))
    }

    /// The STORED Dissolve coverage mask (the CP-0.5 render-companion
    /// standing item): full 256x256 tile resolution, one byte per pixel —
    /// 1 = the pixel took the HARDENED SOURCE (un-premultiplied Cs at
    /// alpha 1, Appendix A), 0 = the backdrop pixel (incl. the sa==0
    /// hardening rule and exact source==backdrop coincidences, which
    /// classify 0 deterministically). Pins the deterministic screen-door
    /// pattern itself as golden bytes, beyond the membership property.
    pub fn dissolve_coverage_mask() -> Vec<u8> {
        let src = build_tile(TileDef::SrcRamp);
        let bd = build_tile(TileDef::BackdropRamp);
        let mut dst = build_tile(TileDef::BackdropRamp);
        composite_tile(&mut dst, &src, BlendMode::Dissolve, 1.0)
            .expect("dissolve composites (zero-ctx door)");
        let eq = |a: [f32; 4], c: [f32; 4]| a.iter().zip(c).all(|(x, y)| (x - y).abs() <= 1e-6);
        let mut mask = Vec::with_capacity(256 * 256);
        for y in 0..256usize {
            for x in 0..256usize {
                let got = dst.get(x, y);
                let s = src.get(x, y);
                let b = bd.get(x, y);
                let sa = s[3];
                let hardened = if sa > 0.0 {
                    [s[0] / sa, s[1] / sa, s[2] / sa, 1.0]
                } else {
                    b
                };
                assert!(
                    eq(got, hardened) || eq(got, b),
                    "dissolve ({x},{y}): membership violated: {got:?}"
                );
                mask.push(u8::from(eq(got, hardened) && !eq(got, b)));
            }
        }
        mask
    }

    fn window_bytes(get: impl Fn(usize, usize) -> [f32; 4]) -> Vec<u8> {
        let mut out = Vec::with_capacity(WIN * WIN * 4 * 4);
        for wy in 0..WIN {
            for wx in 0..WIN {
                for s in get(wx * STRIDE, wy * STRIDE) {
                    out.extend_from_slice(&s.to_le_bytes());
                }
            }
        }
        out
    }

    /// Document-walk golden: doc.new white 96x64, export PNG, paste it
    /// back as a second layer THROUGH THE JOURNALED PATH (`edit.paste`
    /// over an ingested raster asset — the retired `import_layer` door's
    /// MAJOR-2 replacement; identical decoded pixels, so the pinned
    /// golden bytes are unchanged), composite via [`composite_document`]
    /// — the end-to-end public path (both layers Normal; blend-mode
    /// variety per layer arrives with layer.set_props at its wave).
    /// MECHANICAL companion edit by O-CORE (CP-0.5 wave 1), flagged for
    /// O-QA ratification.
    pub fn doc_walk_window() -> Vec<u8> {
        use appname_core::{export_png_bytes, IVec2};
        let mut engine = crate::assembly_point::assembled_engine();
        engine
            .apply_bytes(
                br#"{"schema":"appname.command.v1","id":"b1e4d000-0000-4000-8000-000000000001","kind":"doc.new","params":{"width":96,"height":64,"background":"white"}}"#,
            )
            .expect("doc.new");
        let png = export_png_bytes(&engine).expect("export");
        let asset = engine.add_raster_asset(png).expect("raster ingest");
        engine
            .apply_bytes(
                format!(
                    r#"{{"schema":"appname.command.v1","id":"b1e4d000-0000-4000-8000-000000000002","kind":"edit.paste","params":{{"assetId":"{asset}","at":[0,0],"newLayerId":"b1e4d000-0000-4000-8000-000000000003"}}}}"#,
                )
                .as_bytes(),
            )
            .expect("journaled paste-back");
        let composite = composite_document(
            engine.document().unwrap(),
            IRect::new(0, 0, WIN as i32, WIN as i32),
        )
        .expect("composite_document");
        let region = composite.region_rgba_f32(IRect::new(0, 0, WIN as i32, WIN as i32));
        let mut out = Vec::with_capacity(region.len() * 4);
        for s in region {
            out.extend_from_slice(&s.to_le_bytes());
        }
        // sanity anchored in the meta: white over white stays opaque white
        assert_eq!(
            composite.pixel(IVec2::new(0, 0)),
            [1.0, 1.0, 1.0, 1.0],
            "doc-walk composite must be opaque white"
        );
        out
    }

    /// HAND-COMPUTED oracle probes: (case, window x, window y, expected
    /// un-premultiplied-free stored Co (each of R=G=B), expected αo).
    ///
    /// Derivation (2026-08-12, O-QA): scalar evaluation of the §4.4
    /// formula with the §4.5 blend functions, performed independently of
    /// appname-render's code (calculation script recorded in
    /// reports/O-QA-log.md; full steps per case in the .meta files).
    /// Window (wx,wy) samples tile pixel (4·wx, 4·wy); ramp inputs are
    /// exact k/255 values. Tolerance in the verify test: 2e-5 absolute
    /// (f64 derivation vs f32 pipeline).
    pub const PROBES: &[(&str, usize, usize, f32, f32)] = &[
        // spec worked example — the NORMATIVE §4.4 numbers
        ("spec-worked-multiply", 0, 0, 0.38, 0.90),
        ("spec-worked-multiply", 63, 63, 0.38, 0.90),
        ("spec-worked-normal", 0, 0, 0.50, 0.90),
        // per-mode ramp probes at window (32,16) -> tile (128,64):
        //   Cs=128/255, sa=64/255, Cb=64/255, ab=128/255
        // and window (63,63) -> tile (252,252): all = 252/255
        ("mode-normal", 32, 16, 0.220345538, 0.626958862),
        ("mode-normal", 63, 63, 0.988098514, 0.999861592),
        ("mode-darken", 32, 16, 0.188726448, 0.626958862),
        ("mode-multiply", 32, 16, 0.172978900, 0.626958862),
        ("mode-multiply", 63, 63, 0.976744167, 0.999861592),
        ("mode-lighten", 32, 16, 0.220345538, 0.626958862),
        ("mode-screen", 32, 16, 0.236093085, 0.626958862),
        ("mode-linear_dodge", 32, 16, 0.251964629, 0.626958862),
        ("mode-overlay", 32, 16, 0.188850444, 0.626958862),
        ("mode-difference", 32, 16, 0.188726448, 0.626958862),
        ("mode-difference", 63, 63, 0.022979035, 0.999861592),
        // opacity stack (normal 0.5 then 0.25)
        ("opacity-stack-normal", 32, 16, 0.193794317, 0.591787834),
        ("opacity-stack-normal", 63, 63, 0.983806841, 0.995518828),
        // a=0: zeros, and the un-premultiply guard yields no NaN
        ("alpha-zero", 32, 16, 0.0, 0.0),
        ("alpha-zero", 63, 63, 0.0, 0.0),
        // HDR: >1 passes through; screen goes NEGATIVE with HDR inputs
        // (the formula's honest output); linear_dodge clamps B at 1
        ("hdr-normal", 63, 63, 3.929415021, 0.999861592),
        ("hdr-multiply", 63, 63, 7.699058163, 0.999861592),
        ("hdr-screen", 63, 63, -1.770467078, 0.999861592),
        ("hdr-linear_dodge", 63, 63, 1.045546102, 0.999861592),
        ("hdr-linear_dodge", 32, 16, 0.565685294, 0.626958862),
        // 4-layer mixed stack
        ("stack-mixed-4", 32, 16, 0.267893148, 0.751795703),
        ("stack-mixed-4", 63, 63, 0.752636900, 0.999986355),
    ];

    /// Per-channel oracles for the CP-0.5 chromatic cases:
    /// (case, wx, wy, [r,g,b] stored Co, ao). Same independent
    /// derivation discipline (Appendix A formula text + the §4.4
    /// wrapper; scalar evaluation recorded in the lane log).
    pub const PROBES_RGB: &[(&str, usize, usize, [f32; 3], f32)] = &[
        (
            "mode2-color_burn",
            32,
            16,
            [0.094156863, 0.065254902, 0.110352941],
            0.98,
        ),
        (
            "mode2-color_burn",
            63,
            63,
            [0.343058824, 0.968369748, 0.884],
            0.98,
        ),
        (
            "mode2-linear_burn",
            32,
            16,
            [0.094156863, 0.065254902, 0.110352941],
            0.98,
        ),
        (
            "mode2-linear_burn",
            63,
            63,
            [0.340588235, 0.96, 0.369411765],
            0.98,
        ),
        (
            "mode2-color_dodge",
            32,
            16,
            [0.52785765, 0.306511446, 0.592235294],
            0.98,
        ),
        (
            "mode2-color_dodge",
            63,
            63,
            [0.853058824, 0.976941176, 0.917882353],
            0.98,
        ),
        (
            "mode2-soft_light",
            32,
            16,
            [0.310856315, 0.178550056, 0.38176609],
            0.98,
        ),
        (
            "mode2-soft_light",
            63,
            63,
            [0.523222353, 0.972593993, 0.905226298],
            0.98,
        ),
        (
            "mode2-hard_light",
            32,
            16,
            [0.312133333, 0.155962168, 0.291058824],
            0.98,
        ),
        (
            "mode2-hard_light",
            63,
            63,
            [0.8412, 0.976741869, 0.553647059],
            0.98,
        ),
        (
            "mode2-vivid_light",
            32,
            16,
            [0.311007256, 0.065254902, 0.113176471],
            0.98,
        ),
        (
            "mode2-vivid_light",
            63,
            63,
            [0.853058824, 0.976941176, 0.900941176],
            0.98,
        ),
        (
            "mode2-linear_light",
            32,
            16,
            [0.312980392, 0.065254902, 0.111764706],
            0.98,
        ),
        (
            "mode2-linear_light",
            63,
            63,
            [0.853058824, 0.976941176, 0.549411765],
            0.98,
        ),
        (
            "mode2-pin_light",
            32,
            16,
            [0.310156863, 0.245960784, 0.470352941],
            0.98,
        ),
        (
            "mode2-pin_light",
            63,
            63,
            [0.836117647, 0.968470588, 0.557882353],
            0.98,
        ),
        (
            "mode2-hard_mix",
            32,
            16,
            [0.094156863, 0.065254902, 0.110352941],
            0.98,
        ),
        (
            "mode2-hard_mix",
            63,
            63,
            [0.853058824, 0.976941176, 0.917882353],
            0.98,
        ),
        (
            "mode2-exclusion",
            32,
            16,
            [0.454721569, 0.3359594, 0.471058824],
            0.98,
        ),
        (
            "mode2-exclusion",
            63,
            63,
            [0.633670588, 0.273683045, 0.733647059],
            0.98,
        ),
        (
            "mode2-subtract",
            32,
            16,
            [0.094156863, 0.065254902, 0.291764706],
            0.98,
        ),
        (
            "mode2-subtract",
            63,
            63,
            [0.133058824, 0.256941176, 0.729411765],
            0.98,
        ),
        (
            "mode2-divide",
            32,
            16,
            [0.524469363, 0.785254902, 0.830352941],
            0.98,
        ),
        (
            "mode2-divide",
            63,
            63,
            [0.351630252, 0.976941176, 0.917882353],
            0.98,
        ),
        (
            "mode2-darker_color",
            32,
            16,
            [0.310156863, 0.245960784, 0.471764706],
            0.98,
        ),
        (
            "mode2-darker_color",
            63,
            63,
            [0.349058824, 0.968470588, 0.909411765],
            0.98,
        ),
        (
            "mode2-lighter_color",
            32,
            16,
            [0.455568627, 0.245960784, 0.290352941],
            0.98,
        ),
        (
            "mode2-lighter_color",
            63,
            63,
            [0.844588235, 0.968470588, 0.377882353],
            0.98,
        ),
        (
            "mode2-hue",
            32,
            16,
            [0.431407895, 0.222503188, 0.266898091],
            0.98,
        ),
        (
            "mode2-hue",
            63,
            63,
            [0.750437647, 0.87432, 0.319731765],
            0.98,
        ),
        (
            "mode2-saturation",
            32,
            16,
            [0.310175723, 0.245841777, 0.472351581],
            0.98,
        ),
        (
            "mode2-saturation",
            63,
            63,
            [0.329294118, 0.976941176, 0.917882353],
            0.98,
        ),
        (
            "mode2-color",
            32,
            16,
            [0.431900392, 0.222292549, 0.266684706],
            0.98,
        ),
        (
            "mode2-color",
            63,
            63,
            [0.754397647, 0.87828, 0.287691765],
            0.98,
        ),
        (
            "mode2-luminosity",
            32,
            16,
            [0.333825098, 0.26962902, 0.495432941],
            0.98,
        ),
        (
            "mode2-luminosity",
            63,
            63,
            [0.629929412, 0.976941176, 0.917882353],
            0.98,
        ),
    ];
}

pub mod assembly_point {
    //! The ASSEMBLED engine construction (integration-log wave-2
    //! arbitration #1; contracts brush.stroke.yaml / eraser.stroke.yaml;
    //! ES §40.5). Since the arbitration-1 lift (f605a40) the stroke
    //! KindImpls live in the `appname_tools::kind_impls` LIB module —
    //! imported plainly here; the slice-B `#[path]` include of the CLI's
    //! (now deleted) `stroke_kinds.rs` is retired, and OQ-9 is
    //! resolved-by-lift. The lifted module carries its own test battery
    //! inside `appname-tools`, so this crate no longer re-runs it
    //! (coverage unchanged workspace-wide).

    pub use appname_tools::kind_impls as stroke_kinds;
    pub use appname_tools::kind_impls::{
        assembly, preset_hard_round_16_id, preset_soft_round_pressure_id,
    };

    /// A freshly built ASSEMBLED engine — the in-process counterpart of
    /// the CLI binary, carrying the SHIPPED composition. Since core
    /// door slice 2 (item D, ratified this round) the chain is not
    /// narrated here: it IS `appname_assembly::PRODUCTION_CHAIN` (the
    /// one manifest), and identity with appname-cli/appname-cabi holds
    /// BY CONSTRUCTION — all five consumers call
    /// `production_assembly()`, the call-site enumeration test reds a
    /// reappearing hand chain, and a future wrap (appname-text's
    /// `with_text_layout` at its wiring slice) reaches every surface,
    /// this one included, as one manifest row.
    pub fn assembled_engine() -> appname_core::Engine {
        assembled_assembly().build()
    }

    /// The same shipped chain, UNBUILT — for reopen paths that need the
    /// assembly itself (`load_package_bytes_with`), keeping every
    /// consumer on one definition of "the shipped composition".
    /// [MECHANICAL CROSS-FENCE COMPANION, flagged for O-QA ratification
    /// (core door slice 2, item D — the single-source assembly
    /// constructor): "one definition of the shipped composition" is now
    /// LITERAL — the chain lives in
    /// `appname_assembly::PRODUCTION_CHAIN` and its parity test
    /// enumerates this call site. In-process ≡ shipped binary by
    /// construction.]
    pub fn assembled_assembly() -> appname_core::EngineAssembly {
        appname_assembly::production_assembly()
    }
}

pub mod stroke_goldens {
    //! Stroke goldens — engine-produced through the ASSEMBLED engine
    //! (CP-0.3 slice B), fixed samples + seeds recorded beside outputs.
    //!
    //! Spec: contracts brush.stroke.yaml / eraser.stroke.yaml; ES §15
    //! (brush engine via appname-tools), §40.5 stage map; plan §0.6 rule
    //! 6; goldens/README.md rules. A LOCK golden is deliberately absent:
    //! no public surface sets §11.1 locks until layer.set_props lands
    //! (its milestone wave) — the `error.command.locked` wire id is
    //! pinned in [`lock_wire_id_is_pinned`] evidence instead (recorded
    //! decision, lane log).

    use crate::assembly_point;
    use appname_core::{export_png_bytes, save_package_bytes, Engine};
    use appname_tools::brush::{encode_samples, InputSample};

    /// QA's own fixed samples: 5 points on a diagonal with a pressure
    /// ramp (distinct from the selftest's 3-point set; both recorded).
    pub fn fixed_samples_basic() -> Vec<u8> {
        let pts: [(f64, f64, f32, f64); 5] = [
            (12.0, 12.0, 0.30, 0.00),
            (36.0, 28.0, 0.55, 0.04),
            (64.0, 48.0, 0.85, 0.08),
            (92.0, 66.0, 0.65, 0.12),
            (116.0, 84.0, 0.40, 0.16),
        ];
        let samples: Vec<InputSample> = pts
            .iter()
            .map(|(x, y, pressure, t)| InputSample {
                x: *x,
                y: *y,
                pressure: *pressure,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                velocity: 0.0,
                t: *t,
            })
            .collect();
        encode_samples(&samples).expect("basic samples encode")
    }

    pub struct StrokeGolden {
        pub name: &'static str,
        pub package: Vec<u8>,
        pub export: Vec<u8>,
        /// Meta lines documenting the fixed inputs + any in-production
        /// evidence (undo/redo byte identity for the undo case).
        pub meta: String,
    }

    fn new_doc(engine: &mut Engine, id_tail: u32, bg: &str) -> String {
        let env = format!(
            r#"{{"schema":"appname.command.v1","id":"570de000-0000-4000-8000-{id_tail:012}","kind":"doc.new","params":{{"width":128,"height":96,"background":"{bg}"}}}}"#
        );
        let receipt = engine.apply_bytes(env.as_bytes()).expect("golden doc.new");
        let layer = receipt.created_ids.last().copied().expect("bg layer");
        format!("{layer}")
    }

    #[allow(clippy::too_many_arguments)] // fixture builder mirrors the
                                         // envelope's own param surface; QA-tree hygiene round (mechanical)
    fn brush_env(
        id_tail: u32,
        layer: &str,
        preset: uuid::Uuid,
        samples_id: uuid::Uuid,
        color: &str,
        seed: u32,
        opacity: f32,
        size_px: Option<f32>,
    ) -> String {
        let size = size_px
            .map(|s| format!(r#","sizePx":{s}"#))
            .unwrap_or_default();
        format!(
            r#"{{"schema":"appname.command.v1","id":"570de000-0000-4000-8000-{id_tail:012}","kind":"brush.stroke","target":{{"layerId":"{layer}"}},"params":{{"brushPreset":"{preset}","color":{color},"seed":{seed},"opacity":{opacity}{size}}},"samples":"{samples_id}"}}"#
        )
    }

    /// Produce the four stroke goldens (both kinds; the undo case carries
    /// its byte-identity evidence in meta; the lock case is the recorded
    /// absence documented above).
    pub fn produce_all() -> Vec<StrokeGolden> {
        let mut out = Vec::new();

        // 1. brush, hard-round-16, red, seed 0, opacity 1
        {
            let mut engine = assembly_point::assembled_engine();
            let samples_id = engine.add_samples_asset(fixed_samples_basic());
            let layer = new_doc(&mut engine, 1, "white");
            let env = brush_env(
                2,
                &layer,
                assembly_point::preset_hard_round_16_id(),
                samples_id,
                r#"{"model":"rgb","comps":[1,0,0,0]}"#,
                0,
                1.0,
                None,
            );
            engine.apply_bytes(env.as_bytes()).expect("brush hard");
            out.push(StrokeGolden {
                name: "stroke-brush-hard",
                package: save_package_bytes(&engine).unwrap(),
                export: export_png_bytes(&engine).unwrap(),
                meta: format!(
                    "inputs: doc.new 128x96 white (id ...0001); samples = \
                     fixed_samples_basic (5-point diagonal, pressure \
                     ramp); brush.stroke id ...0002 preset hard-round-16 \
                     ({}) color rgb[1,0,0,0] seed 0 opacity 1\n",
                    assembly_point::preset_hard_round_16_id()
                ),
            });
        }

        // 2. brush, soft-round-pressure, green, seed 3, opacity 0.8,
        //    sizePx 24
        {
            let mut engine = assembly_point::assembled_engine();
            let samples_id = engine.add_samples_asset(fixed_samples_basic());
            let layer = new_doc(&mut engine, 3, "white");
            let env = brush_env(
                4,
                &layer,
                assembly_point::preset_soft_round_pressure_id(),
                samples_id,
                r#"{"model":"rgb","comps":[0,0.8,0.1,0]}"#,
                3,
                0.8,
                Some(24.0),
            );
            engine.apply_bytes(env.as_bytes()).expect("brush soft");
            out.push(StrokeGolden {
                name: "stroke-brush-soft",
                package: save_package_bytes(&engine).unwrap(),
                export: export_png_bytes(&engine).unwrap(),
                meta: format!(
                    "inputs: doc.new 128x96 white (id ...0003); samples = \
                     fixed_samples_basic; brush.stroke id ...0004 preset \
                     soft-round-pressure ({}) color rgb[0,0.8,0.1,0] seed \
                     3 opacity 0.8 sizePx 24\n",
                    assembly_point::preset_soft_round_pressure_id()
                ),
            });
        }

        // 3. brush then eraser across it
        {
            let mut engine = assembly_point::assembled_engine();
            let samples_id = engine.add_samples_asset(fixed_samples_basic());
            let layer = new_doc(&mut engine, 5, "black");
            let brush = brush_env(
                6,
                &layer,
                assembly_point::preset_hard_round_16_id(),
                samples_id,
                r#"{"model":"rgb","comps":[0.2,0.4,1,0]}"#,
                1,
                1.0,
                None,
            );
            engine.apply_bytes(brush.as_bytes()).expect("brush base");
            let eraser = format!(
                r#"{{"schema":"appname.command.v1","id":"570de000-0000-4000-8000-{:012}","kind":"eraser.stroke","target":{{"layerId":"{layer}"}},"params":{{"brushPreset":"{}","seed":2,"opacity":0.9,"sizePx":20}},"samples":"{samples_id}"}}"#,
                7,
                assembly_point::preset_hard_round_16_id(),
            );
            engine.apply_bytes(eraser.as_bytes()).expect("eraser");
            out.push(StrokeGolden {
                name: "stroke-eraser-over-brush",
                package: save_package_bytes(&engine).unwrap(),
                export: export_png_bytes(&engine).unwrap(),
                meta: "inputs: doc.new 128x96 black (id ...0005); brush \
                       id ...0006 hard-round-16 rgb[0.2,0.4,1,0] seed 1; \
                       eraser.stroke id ...0007 hard-round-16 seed 2 \
                       opacity 0.9 sizePx 20 over the SAME samples path \
                       (erases along the stroke)\n"
                    .to_string(),
            });
        }

        // 4. undo case: stroke -> undo -> redo, byte identities proven at
        //    production time and re-proven by the verify test
        {
            let mut engine = assembly_point::assembled_engine();
            let samples_id = engine.add_samples_asset(fixed_samples_basic());
            let layer = new_doc(&mut engine, 8, "white");
            let base_export = export_png_bytes(&engine).unwrap();
            let base_hash = engine.state_hash();
            let env = brush_env(
                9,
                &layer,
                assembly_point::preset_hard_round_16_id(),
                samples_id,
                r#"{"model":"rgb","comps":[0.6,0,0.6,0]}"#,
                5,
                1.0,
                None,
            );
            engine.apply_bytes(env.as_bytes()).expect("brush undo-case");
            let stroked_export = export_png_bytes(&engine).unwrap();
            let stroked_hash = engine.state_hash();
            assert_ne!(
                base_export, stroked_export,
                "undo-case stroke must change the export"
            );
            engine.undo().expect("undo stroke");
            assert_eq!(
                export_png_bytes(&engine).unwrap(),
                base_export,
                "undo must restore the pre-stroke export byte-exactly"
            );
            assert_eq!(engine.state_hash(), base_hash);
            engine.redo().expect("redo stroke");
            assert_eq!(
                export_png_bytes(&engine).unwrap(),
                stroked_export,
                "redo must restore the stroked export byte-exactly"
            );
            assert_eq!(engine.state_hash(), stroked_hash);
            out.push(StrokeGolden {
                name: "stroke-undo-redo",
                package: save_package_bytes(&engine).unwrap(),
                export: stroked_export,
                meta: format!(
                    "inputs: doc.new 128x96 white (id ...0008); brush id \
                     ...0009 hard-round-16 rgb[0.6,0,0.6,0] seed 5\n\
                     evidence: stroke changes the export; undo restores \
                     the pre-stroke export AND state_hash byte-exactly; \
                     redo restores the stroked export AND state_hash \
                     ({}) — asserted at production AND on every cargo \
                     test run (verify suite)\n",
                    engine.state_hash()
                ),
            });
        }

        out
    }
}

pub mod edit_goldens {
    //! fill/paste goldens — engine-produced through the ASSEMBLED engine
    //! (CP-0.5 wave 2), per the stroke-golden pattern.
    //!
    //! Spec: contracts fill.yaml / edit.paste.yaml; O-CORE CP-0.5 wave-1
    //! entry (§40.5 stage maps; journaled paste over ingested raster
    //! assets); plan §0.6 rule 6. Deferred WITH REASONS (recorded here
    //! and in the lane log): an edit.paste_into accept golden needs a
    //! live selection (selection.set is milestone-0.5 pending — its
    //! selection_required gate is asserted as EVIDENCE in the verify
    //! suite instead); an edit.cut accept golden waits for the same
    //! selection wave to exercise its selection semantics deliberately.

    use crate::assembly_point;
    use appname_core::{export_png_bytes, save_package_bytes, Engine};

    pub struct EditGolden {
        pub name: &'static str,
        pub package: Vec<u8>,
        pub export: Vec<u8>,
        pub meta: String,
    }

    fn new_doc(engine: &mut Engine, id_tail: u32, w: u32, h: u32, bg: &str) -> String {
        let env = format!(
            r#"{{"schema":"appname.command.v1","id":"ed17e000-0000-4000-8000-{id_tail:012}","kind":"doc.new","params":{{"width":{w},"height":{h},"background":"{bg}"}}}}"#
        );
        let receipt = engine.apply_bytes(env.as_bytes()).expect("golden doc.new");
        format!("{}", receipt.created_ids.last().expect("bg layer"))
    }

    /// A small deterministic black PNG (32x24), engine-exported — the
    /// paste-source asset.
    pub fn paste_source_png() -> Vec<u8> {
        let mut e = Engine::new();
        e.apply_bytes(
            br#"{"schema":"appname.command.v1","id":"ed17e000-0000-4000-8000-000000000099","kind":"doc.new","params":{"width":32,"height":24,"background":"black"}}"#,
        )
        .expect("paste source doc");
        export_png_bytes(&e).expect("paste source png")
    }

    pub fn produce_all() -> Vec<EditGolden> {
        let mut out = Vec::new();

        // 1. fill foreground on white
        {
            let mut engine = assembly_point::assembled_engine();
            let layer = new_doc(&mut engine, 1, 128, 96, "white");
            let env = format!(
                r#"{{"schema":"appname.command.v1","id":"ed17e000-0000-4000-8000-000000000002","kind":"fill","target":{{"layerId":"{layer}"}},"params":{{"content":"foreground"}}}}"#
            );
            engine.apply_bytes(env.as_bytes()).expect("fill fg");
            out.push(EditGolden {
                name: "fill-foreground",
                package: save_package_bytes(&engine).unwrap(),
                export: export_png_bytes(&engine).unwrap(),
                meta: "inputs: doc.new 128x96 white (id ...0001); fill id \
                       ...0002 content foreground on the background layer\n"
                    .to_string(),
            });
        }

        // 2. fill background-content on black
        {
            let mut engine = assembly_point::assembled_engine();
            let layer = new_doc(&mut engine, 3, 128, 96, "black");
            let env = format!(
                r#"{{"schema":"appname.command.v1","id":"ed17e000-0000-4000-8000-000000000004","kind":"fill","target":{{"layerId":"{layer}"}},"params":{{"content":"background"}}}}"#
            );
            engine.apply_bytes(env.as_bytes()).expect("fill bg");
            out.push(EditGolden {
                name: "fill-background-content",
                package: save_package_bytes(&engine).unwrap(),
                export: export_png_bytes(&engine).unwrap(),
                meta: "inputs: doc.new 128x96 black (id ...0003); fill id \
                       ...0004 content background on the background layer\n"
                    .to_string(),
            });
        }

        // 3. paste-basic: ingested raster asset -> journaled edit.paste
        {
            let mut engine = assembly_point::assembled_engine();
            new_doc(&mut engine, 5, 128, 96, "white");
            let asset = engine
                .add_raster_asset(paste_source_png())
                .expect("ingest paste source");
            let env = format!(
                r#"{{"schema":"appname.command.v1","id":"ed17e000-0000-4000-8000-000000000006","kind":"edit.paste","params":{{"assetId":"{asset}","at":[16,12],"newLayerId":"ed17e000-0000-4000-8000-000000000007"}}}}"#
            );
            engine.apply_bytes(env.as_bytes()).expect("paste");
            out.push(EditGolden {
                name: "paste-basic",
                package: save_package_bytes(&engine).unwrap(),
                export: export_png_bytes(&engine).unwrap(),
                meta: format!(
                    "inputs: doc.new 128x96 white (id ...0005); ingested \
                     raster asset {asset} (engine-exported 32x24 black \
                     png); edit.paste id ...0006 at [16,12] newLayerId \
                     ...0007 (journaled import path)\n"
                ),
            });
        }

        // 4. paste-undo-redo evidence case
        {
            let mut engine = assembly_point::assembled_engine();
            new_doc(&mut engine, 8, 128, 96, "white");
            let asset = engine.add_raster_asset(paste_source_png()).expect("ingest");
            let base_export = export_png_bytes(&engine).unwrap();
            let base_hash = engine.state_hash();
            let env = format!(
                r#"{{"schema":"appname.command.v1","id":"ed17e000-0000-4000-8000-000000000009","kind":"edit.paste","params":{{"assetId":"{asset}","at":[40,30],"newLayerId":"ed17e000-0000-4000-8000-00000000000a"}}}}"#
            );
            engine.apply_bytes(env.as_bytes()).expect("paste undo-case");
            let pasted_export = export_png_bytes(&engine).unwrap();
            let pasted_hash = engine.state_hash();
            assert_ne!(base_export, pasted_export, "paste must change export");
            engine.undo().expect("undo paste");
            assert_eq!(export_png_bytes(&engine).unwrap(), base_export);
            assert_eq!(engine.state_hash(), base_hash);
            engine.redo().expect("redo paste");
            assert_eq!(export_png_bytes(&engine).unwrap(), pasted_export);
            assert_eq!(engine.state_hash(), pasted_hash);
            out.push(EditGolden {
                name: "paste-undo-redo",
                package: save_package_bytes(&engine).unwrap(),
                export: pasted_export,
                meta: format!(
                    "inputs: doc.new 128x96 white (id ...0008); asset \
                     {asset}; edit.paste id ...0009 at [40,30]\n\
                     evidence: paste changes the export; undo restores \
                     pre-paste export AND state_hash byte-exactly; redo \
                     restores both ({}) — asserted at production AND on \
                     every cargo test run\n",
                    engine.state_hash()
                ),
            });
        }

        out
    }
}

pub mod color_mask_goldens {
    //! Mask + color goldens (CP-0.5 wave 3) — engine-produced through the
    //! ASSEMBLED engine, per the established pattern.
    //!
    //! Spec: contracts mask.add.yaml / doc.assign_profile.yaml /
    //! doc.convert_profile.yaml; O-CORE wave-3 entries (§13/§40.3 masks;
    //! §5 color wiring: assign = zero-delta retag, convert = real pixel
    //! transform); plan §0.6 rule 6. The assign-vs-convert pair is the
    //! W04 core as GOLDEN BYTES: same base document, same target profile
    //! (builtin DisplayP3) — assign leaves every raster `region_hash`
    //! untouched, convert moves the stroked layer's (saturated content;
    //! neutrals would legitimately no-op).

    use crate::assembly_point;
    use crate::stroke_goldens::fixed_samples_basic;
    use appname_core::{export_png_bytes, save_package_bytes, ContentHash, Engine};

    pub fn display_p3_id() -> appname_core::graph::id::ProfileId {
        appname_core::engine::color_glue::builtin_profile_id(appname_color::BuiltinSpace::DisplayP3)
    }

    pub struct Golden {
        pub name: &'static str,
        pub package: Vec<u8>,
        pub export: Option<Vec<u8>>,
        pub meta: String,
    }

    /// Base document shared by the color pair: 160x120 white + one
    /// saturated red stroke (fixed ids/seed).
    fn color_base() -> Engine {
        let mut engine = assembly_point::assembled_engine();
        let receipt = engine
            .apply_bytes(
                br#"{"schema":"appname.command.v1","id":"c0104000-0000-4000-8000-000000000001","kind":"doc.new","params":{"width":160,"height":120,"background":"white"}}"#,
            )
            .expect("color base doc");
        let layer = receipt.created_ids.last().copied().unwrap();
        let samples = engine.add_samples_asset(fixed_samples_basic());
        let env = format!(
            r#"{{"schema":"appname.command.v1","id":"c0104000-0000-4000-8000-000000000002","kind":"brush.stroke","target":{{"layerId":"{layer}"}},"params":{{"brushPreset":"{preset}","color":{{"model":"rgb","comps":[1,0.2,0,0]}},"seed":11,"opacity":1}},"samples":"{samples}"}}"#,
            preset = assembly_point::preset_hard_round_16_id(),
        );
        engine
            .apply_bytes(env.as_bytes())
            .expect("saturated stroke");
        engine
    }

    fn raster_hashes(engine: &Engine) -> Vec<ContentHash> {
        let doc = engine.document().expect("doc");
        doc.z_order_walk()
            .into_iter()
            .filter_map(|id| doc.layer(id).unwrap().pixels())
            .map(|b| b.region_hash())
            .collect()
    }

    pub fn produce_all() -> Vec<Golden> {
        let mut out = Vec::new();

        // 1. mask round-trip: pasted layer + mask.add, save->load->save
        //    byte identity, undo/redo evidence
        {
            let mut engine = assembly_point::assembled_engine();
            engine
                .apply_bytes(
                    br#"{"schema":"appname.command.v1","id":"c0104000-0000-4000-8000-000000000010","kind":"doc.new","params":{"width":96,"height":64,"background":"white"}}"#,
                )
                .expect("mask doc");
            let asset = engine
                .add_raster_asset(crate::edit_goldens::paste_source_png())
                .expect("ingest");
            let paste = format!(
                r#"{{"schema":"appname.command.v1","id":"c0104000-0000-4000-8000-000000000011","kind":"edit.paste","params":{{"assetId":"{asset}","at":[8,8],"newLayerId":"c0104000-0000-4000-8000-000000000012"}}}}"#
            );
            engine.apply_bytes(paste.as_bytes()).expect("paste");
            let pre_mask_hash = engine.state_hash();
            let mask = r#"{"schema":"appname.command.v1","id":"c0104000-0000-4000-8000-000000000013","kind":"mask.add","target":{"layerId":"c0104000-0000-4000-8000-000000000012"},"params":{"kind":"raster","from":"reveal_all","newMaskId":"c0104000-0000-4000-8000-000000000014"}}"#;
            engine.apply_bytes(mask.as_bytes()).expect("mask.add");
            let masked_hash = engine.state_hash();

            // round-trip: save -> load -> save byte identity
            let pkg = save_package_bytes(&engine).unwrap();
            let loaded = appname_core::load_package_bytes(&pkg).expect("golden package reopens");
            assert_eq!(
                save_package_bytes(&loaded.engine).unwrap(),
                pkg,
                "mask package must round-trip byte-identically"
            );
            assert_eq!(loaded.engine.state_hash(), masked_hash);

            // undo/redo evidence
            engine.undo().expect("undo mask.add");
            assert_eq!(engine.state_hash(), pre_mask_hash);
            engine.redo().expect("redo mask.add");
            assert_eq!(engine.state_hash(), masked_hash);

            // §11.2 Sub-slice A (O-RENDER v1.0.1): the aud-replay F2 stopgap
            // is RETIRED — an unfeathered raster mask now SERVES on both
            // reference doors (never refused, never silently dropped). A
            // `reveal_all` mask is the §11.2 no-op, so the export is the
            // correct composited bytes (the pasted layer over the white
            // background), pinned here again. Both reference doors agree on
            // these bytes (one compositing truth).
            let export = export_png_bytes(&engine).expect("masked doc now serves export");
            let (rw, rh, plane) =
                appname_core::composite_plane_f32(&engine).expect("readback serves");
            assert_eq!(
                appname_core::encode_composite_png(rw, rh, &plane).unwrap(),
                export,
                "export and f32 readback are one compositing truth on the masked doc"
            );
            out.push(Golden {
                name: "mask-add-roundtrip",
                package: pkg,
                export: Some(export),
                meta: format!(
                    "inputs: doc.new 96x64 white (id ...0010); pasted \
                     32x24 layer (id ...0012); mask.add raster reveal_all \
                     newMaskId ...0014\n\
                     evidence: save->load->save byte-identical; loaded \
                     state_hash == produced ({masked_hash}); undo removes \
                     the mask (state_hash restored), redo re-attaches; \
                     §11.2 Sub-slice A serves the mask — export pins the \
                     correct composited bytes (reveal_all is the no-op; \
                     export == f32 readback, one compositing truth)\n"
                ),
            });
        }

        // 1b. masks_stack (§11.2 density sweep, contract golden ref
        //     `masks_stack.appname`): a pasted layer over white with a
        //     hide_all mask at density 0.5 — the layer composites at HALF
        //     coverage (E = 1 − 0.5·(1 − 0) = 0.5), so the export is
        //     neither the unmasked paste NOR pure white: the mask is
        //     genuinely APPLIED (the anti-F2 proof, as GOLDEN BYTES).
        {
            let mut engine = assembly_point::assembled_engine();
            engine
                .apply_bytes(
                    br#"{"schema":"appname.command.v1","id":"c0104000-0000-4000-8000-000000000020","kind":"doc.new","params":{"width":96,"height":64,"background":"white"}}"#,
                )
                .expect("masks_stack doc");
            let asset = engine
                .add_raster_asset(crate::edit_goldens::paste_source_png())
                .expect("ingest");
            let paste = format!(
                r#"{{"schema":"appname.command.v1","id":"c0104000-0000-4000-8000-000000000021","kind":"edit.paste","params":{{"assetId":"{asset}","at":[8,8],"newLayerId":"c0104000-0000-4000-8000-000000000022"}}}}"#
            );
            engine.apply_bytes(paste.as_bytes()).expect("paste");
            // The UNMASKED composite (for the "mask changed pixels" proof).
            let unmasked = export_png_bytes(&engine).expect("unmasked serves");
            engine
                .apply_bytes(
                    br#"{"schema":"appname.command.v1","id":"c0104000-0000-4000-8000-000000000023","kind":"mask.add","target":{"layerId":"c0104000-0000-4000-8000-000000000022"},"params":{"kind":"raster","from":"hide_all","newMaskId":"c0104000-0000-4000-8000-000000000024"}}"#,
                )
                .expect("mask.add hide_all");
            engine
                .apply_bytes(
                    br#"{"schema":"appname.command.v1","id":"c0104000-0000-4000-8000-000000000025","kind":"mask.set_props","target":{"layerId":"c0104000-0000-4000-8000-000000000022","maskId":"c0104000-0000-4000-8000-000000000024"},"params":{"density":0.5}}"#,
                )
                .expect("mask.set_props density 0.5");
            let masked_hash = engine.state_hash();
            let pkg = save_package_bytes(&engine).unwrap();
            let loaded = appname_core::load_package_bytes(&pkg).expect("masks_stack reopens");
            assert_eq!(
                save_package_bytes(&loaded.engine).unwrap(),
                pkg,
                "masks_stack package round-trips byte-identically"
            );
            assert_eq!(loaded.engine.state_hash(), masked_hash);
            let export = export_png_bytes(&engine).expect("density-masked doc serves");
            assert_ne!(
                export, unmasked,
                "density 0.5 mask must CHANGE the composited bytes (mask applied, not dropped)"
            );
            // export == f32 readback (one compositing truth).
            let (rw, rh, plane) =
                appname_core::composite_plane_f32(&engine).expect("readback serves");
            assert_eq!(
                appname_core::encode_composite_png(rw, rh, &plane).unwrap(),
                export,
                "export and f32 readback agree on the density-masked composite"
            );
            out.push(Golden {
                name: "masks_stack",
                package: pkg,
                export: Some(export),
                meta: "inputs: doc.new 96x64 white (id ...0020); pasted 32x24 layer \
                       (id ...0022); mask.add raster hide_all newMaskId ...0024; \
                       mask.set_props density 0.5\n\
                       evidence: §11.2 effective coverage E = 1 − density·(1 − c) = 0.5 \
                       over the hide_all coverage (c = 0), so the pasted layer composites \
                       at HALF coverage — the export differs from BOTH the unmasked paste \
                       and pure white (the mask is applied); package round-trips \
                       byte-identically; export == f32 readback (one compositing truth)\n"
                    .to_string(),
            });
        }

        // 2-4. the assign-vs-convert pixel-truth pair over a shared base
        {
            let base = color_base();
            let base_hashes = raster_hashes(&base);
            let base_profile = base.document().unwrap().color().working;
            out.push(Golden {
                name: "color-base",
                package: save_package_bytes(&base).unwrap(),
                export: Some(export_png_bytes(&base).unwrap()),
                meta: "the shared base: 160x120 white + saturated red \
                       stroke (seed 11) — committed so the pair's diffs \
                       are inspectable\n"
                    .to_string(),
            });

            let p3 = display_p3_id();
            // assign: retag only
            let mut assign = color_base();
            let env = format!(
                r#"{{"schema":"appname.command.v1","id":"c0104000-0000-4000-8000-000000000003","kind":"doc.assign_profile","params":{{"profile":"{}"}}}}"#,
                p3.0
            );
            assign.apply_bytes(env.as_bytes()).expect("assign");
            assert_eq!(
                raster_hashes(&assign),
                base_hashes,
                "assign must not touch a single raster region_hash"
            );
            assert_ne!(
                assign.document().unwrap().color().working,
                base_profile,
                "assign must retag the working profile"
            );
            out.push(Golden {
                name: "color-assign-p3",
                package: save_package_bytes(&assign).unwrap(),
                export: None, // interpretation-only change; export policy
                // is the display transform's wave
                meta: format!(
                    "base + doc.assign_profile {{profile: {}}} (builtin \
                     DisplayP3)\n\
                     evidence: EVERY raster region_hash byte-identical to \
                     the base; working profile retagged — assign changes \
                     interpretation, never pixels\n",
                    p3.0
                ),
            });

            // convert: pixels move (saturated content)
            let mut convert = color_base();
            let env = format!(
                r#"{{"schema":"appname.command.v1","id":"c0104000-0000-4000-8000-000000000004","kind":"doc.convert_profile","params":{{"profile":"{}"}}}}"#,
                p3.0
            );
            convert.apply_bytes(env.as_bytes()).expect("convert");
            let conv_hashes = raster_hashes(&convert);
            assert_ne!(
                conv_hashes, base_hashes,
                "convert of saturated content must move pixels"
            );
            // undo restores the exact bytes (delta fidelity across the
            // CMM path)
            convert.undo().expect("undo convert");
            assert_eq!(raster_hashes(&convert), base_hashes);
            convert.redo().expect("redo convert");
            assert_eq!(raster_hashes(&convert), conv_hashes);
            out.push(Golden {
                name: "color-convert-p3",
                package: save_package_bytes(&convert).unwrap(),
                export: Some(export_png_bytes(&convert).unwrap()),
                meta: format!(
                    "base + doc.convert_profile {{profile: {}}} (builtin \
                     DisplayP3)\n\
                     evidence: raster region_hashes MOVED vs the base \
                     (saturated content; neutrals would legitimately \
                     no-op); undo restores the exact base hashes, redo \
                     restores the converted ones — the W04 pixel truth \
                     as golden bytes\n",
                    p3.0
                ),
            });
        }

        out
    }
}

pub mod kind_tail_goldens {
    //! CP-0.5 kind-tail goldens (close ledger): the wave-4 layer family,
    //! the merge trio's pixel truth, and canvas furniture — engine-
    //! produced through the ASSEMBLED engine per the established pattern.
    //!
    //! Spec: contracts layer.*.yaml / image.flatten.yaml / guide.*.yaml /
    //! grid.set.yaml; ES §4 (the merge truth: baking a stack must equal
    //! the compositor's own Normal-reference composite BYTE-EXACTLY on
    //! export), §24.2 (furniture never touches pixels); plan §0.6 rule 6.
    //! doc.duplicate (†, the one still-pending watched arm at rev 10)
    //! deliberately has NO golden — the drift alarm owns it.

    use crate::assembly_point;
    use crate::stroke_goldens::fixed_samples_basic;
    use appname_core::{export_png_bytes, save_package_bytes, Engine};

    pub struct Golden {
        pub name: &'static str,
        pub package: Vec<u8>,
        pub export: Vec<u8>,
        pub meta: String,
    }

    fn doc_with_stroked_l2(tail: u32) -> (Engine, String) {
        // white base + raster layer L2 + saturated stroke on L2
        let mut engine = assembly_point::assembled_engine();
        engine
            .apply_bytes(
                format!(
                    r#"{{"schema":"appname.command.v1","id":"7a11e000-0000-4000-8000-{tail:012}","kind":"doc.new","params":{{"width":160,"height":120,"background":"white"}}}}"#
                )
                .as_bytes(),
            )
            .expect("doc.new");
        let l2 = format!("7a11e000-0000-4000-8000-{:012}", tail + 1);
        engine
            .apply_bytes(
                format!(
                    r#"{{"schema":"appname.command.v1","id":"7a11e000-0000-4000-8000-{:012}","kind":"layer.add","params":{{"kind":"raster","newLayerId":"{l2}"}}}}"#,
                    tail + 2
                )
                .as_bytes(),
            )
            .expect("layer.add");
        let samples = engine.add_samples_asset(fixed_samples_basic());
        engine
            .apply_bytes(
                format!(
                    r#"{{"schema":"appname.command.v1","id":"7a11e000-0000-4000-8000-{:012}","kind":"brush.stroke","target":{{"layerId":"{l2}"}},"params":{{"brushPreset":"{preset}","color":{{"model":"rgb","comps":[1,0.2,0,0]}},"seed":11,"opacity":1}},"samples":"{samples}"}}"#,
                    tail + 3,
                    preset = assembly_point::preset_hard_round_16_id(),
                )
                .as_bytes(),
            )
            .expect("stroke on L2");
        (engine, l2)
    }

    fn layer_count(engine: &Engine) -> usize {
        engine.document().unwrap().layer_count()
    }

    pub fn produce_all() -> Vec<Golden> {
        let mut out = Vec::new();

        // 1. layer-family round-trip: add + stroke + set_props + rename
        //    + duplicate, then a FULL undo walk to base and redo back —
        //    state_hash byte identity at every station.
        {
            let (mut engine, l2) = doc_with_stroked_l2(0x100);
            let mut stations = vec![engine.state_hash()];
            for env in [
                format!(
                    r#"{{"schema":"appname.command.v1","id":"7a11e000-0000-4000-8000-000000000110","kind":"layer.set_props","target":{{"layerId":"{l2}"}},"params":{{"opacity":0.5}}}}"#
                ),
                format!(
                    r#"{{"schema":"appname.command.v1","id":"7a11e000-0000-4000-8000-000000000111","kind":"layer.rename","target":{{"layerId":"{l2}"}},"params":{{"name":"Sky"}}}}"#
                ),
                format!(
                    r#"{{"schema":"appname.command.v1","id":"7a11e000-0000-4000-8000-000000000112","kind":"layer.duplicate","target":{{"layerId":"{l2}"}},"params":{{"idMap":{{"{l2}":"7a11e000-0000-4000-8000-000000000113"}}}}}}"#
                ),
            ] {
                engine.apply_bytes(env.as_bytes()).expect("family step");
                stations.push(engine.state_hash());
            }
            let full = *stations.last().unwrap();
            // walk down and back, byte identity per station
            for expect in stations.iter().rev().skip(1) {
                engine.undo().expect("undo family");
                assert_eq!(engine.state_hash(), *expect);
            }
            for expect in stations.iter().skip(1) {
                engine.redo().expect("redo family");
                assert_eq!(engine.state_hash(), *expect);
            }
            assert_eq!(engine.state_hash(), full);
            out.push(Golden {
                name: "layer-family-roundtrip",
                package: save_package_bytes(&engine).unwrap(),
                export: export_png_bytes(&engine).unwrap(),
                meta: "inputs: white 160x120 + layer.add L2 + saturated \
                       stroke + set_props opacity 0.5 + rename 'Sky' + \
                       duplicate (fixed idMap)\n\
                       evidence: full undo walk to base and redo back — \
                       state_hash byte-identical at EVERY station, both \
                       directions\n"
                    .to_string(),
            });
        }

        // 2-4. the merge trio's pixel truth: bake == the compositor's own
        //      Normal reference, byte-exact on export
        for (name, tail, env_fmt) in [
            (
                "merge-down-pixel-truth",
                0x200u32,
                r#"{"schema":"appname.command.v1","id":"7a11e000-0000-4000-8000-{id:012}","kind":"layer.merge_down","target":{"layerId":"{l2}"}}"#,
            ),
            (
                "merge-visible-pixel-truth",
                0x300,
                r#"{"schema":"appname.command.v1","id":"7a11e000-0000-4000-8000-{id:012}","kind":"layer.merge_visible","params":{}}"#,
            ),
            (
                "flatten-pixel-truth",
                0x400,
                r#"{"schema":"appname.command.v1","id":"7a11e000-0000-4000-8000-{id:012}","kind":"image.flatten","params":{}}"#,
            ),
        ] {
            let (mut engine, l2) = doc_with_stroked_l2(tail);
            let reference = export_png_bytes(&engine).unwrap();
            let pre_layers = layer_count(&engine);
            let pre_hash = engine.state_hash();
            let env = env_fmt
                .replace("{id:012}", &format!("{:012}", tail + 0x10))
                .replace("{l2}", &l2);
            engine.apply_bytes(env.as_bytes()).expect(name);
            let baked = export_png_bytes(&engine).unwrap();
            // WATCHED DIVERGENCE (flagged to O-CORE, checkpoint evidence):
            // the export path composites in LINEAR premultiplied f32
            // (§4.4) while the CP-0.3-era stroke/merge tile math runs in
            // STORAGE encoding (the recorded interim divergence) — so the
            // bake differs from the pre-merge composite at anti-aliased
            // edges TODAY. The spec truth (§4: merging never changes
            // appearance) re-enters with the §5 linear-light unification;
            // this tripwire fails the workspace the moment the two paths
            // agree, forcing the golden refresh + tightening to byte
            // equality via O-QA review.
            assert_ne!(
                baked, reference,
                "{name}: bake now EQUALS the linear composite — the §5 \
                 unification landed; tighten this golden to byte equality \
                 via O-QA review"
            );
            // and the divergence must stay SMALL (AA-edge-only): measured
            // max channel delta, bounded
            let max_delta = {
                use appname_core::decode_png_to_buffer;
                let working = engine.document().unwrap().color().working;
                let (a, wa, ha) = decode_png_to_buffer(&baked, working).unwrap();
                let (b, wb, hb) = decode_png_to_buffer(&reference, working).unwrap();
                assert_eq!((wa, ha), (wb, hb));
                let mut m = 0f32;
                for y in 0..ha as i32 {
                    for x in 0..wa as i32 {
                        let pa = a.read_pixel_f32(appname_core::IVec2::new(x, y));
                        let pb = b.read_pixel_f32(appname_core::IVec2::new(x, y));
                        for c in 0..4 {
                            m = m.max((pa[c] - pb[c]).abs());
                        }
                    }
                }
                m
            };
            // Measured at install: max delta ~0.41 linear — the interim
            // divergence is LARGE at half-covered saturated edges (this
            // is why the §5 unification matters), not an AA nuance. The
            // envelope guards against it growing further.
            assert!(
                max_delta < 0.5,
                "{name}: storage-vs-linear divergence exceeded the \
                 measured envelope: {max_delta}"
            );
            assert!(
                layer_count(&engine) < pre_layers,
                "{name}: baking reduces the layer count"
            );
            engine.undo().expect("undo merge");
            assert_eq!(engine.state_hash(), pre_hash, "{name}: undo");
            engine.redo().expect("redo merge");
            out.push(Golden {
                name,
                package: save_package_bytes(&engine).unwrap(),
                export: baked,
                meta: format!(
                    "inputs: white 160x120 + stroked L2; then {name} \
                     (fixed ids, tail 0x{tail:x})\n\
                     evidence: WATCHED DIVERGENCE — bake differs from the \
                     linear-composite reference at AA edges (storage-vs-\
                     linear interim, flagged to O-CORE; max channel delta \
                     {max_delta}, envelope < 0.5 — LARGE at saturated \
                     half-covered edges); tripwire flips this golden to \
                     byte equality when the section-5 unification lands; \
                     layer count {pre_layers} -> fewer; undo restores the \
                     pre-merge state_hash byte-exactly\n"
                ),
            });
        }

        // 5. canvas furniture: guides + grid never touch pixels
        {
            let (mut engine, _l2) = doc_with_stroked_l2(0x500);
            let reference = export_png_bytes(&engine).unwrap();
            let pre_hash = engine.state_hash();
            for env in [
                r#"{"schema":"appname.command.v1","id":"7a11e000-0000-4000-8000-000000000510","kind":"guide.add","params":{"orientation":"vertical","position":80}}"#,
                r#"{"schema":"appname.command.v1","id":"7a11e000-0000-4000-8000-000000000511","kind":"guide.add","params":{"orientation":"horizontal","position":60}}"#,
                r#"{"schema":"appname.command.v1","id":"7a11e000-0000-4000-8000-000000000512","kind":"guide.move","params":{"orientation":"vertical","index":0,"position":40.5}}"#,
                r#"{"schema":"appname.command.v1","id":"7a11e000-0000-4000-8000-000000000513","kind":"grid.set","params":{"spacing":32,"subdivisions":4,"show":true}}"#,
            ] {
                engine.apply_bytes(env.as_bytes()).expect("furniture");
            }
            assert_eq!(
                export_png_bytes(&engine).unwrap(),
                reference,
                "furniture must never touch pixels (section 24.2)"
            );
            assert_ne!(
                engine.state_hash(),
                pre_hash,
                "furniture is real document state"
            );
            for _ in 0..4 {
                engine.undo().expect("undo furniture");
            }
            assert_eq!(engine.state_hash(), pre_hash);
            for _ in 0..4 {
                engine.redo().expect("redo furniture");
            }
            out.push(Golden {
                name: "guides-grid-furniture",
                package: save_package_bytes(&engine).unwrap(),
                export: reference,
                meta: "inputs: stroked doc + guide.add x2 + guide.move + \
                       grid.set (fixed values)\n\
                       evidence: export byte-identical before/after \
                       (furniture never touches pixels, section 24.2); \
                       state_hash changes (real state); 4-step undo/redo \
                       round-trip byte-exact\n"
                    .to_string(),
            });
        }

        out
    }
}

pub mod caf_goldens {
    //! CAF §45.3 BEHAVIORAL goldens — the FREEZE the ledger trigger
    //! staged (O-QA CP-1.0 ratification batch).
    //!
    //! FREEZE CITATIONS (the trigger discipline, both required):
    //! 1. "CAF §45.3 SEEDED-STREAM MICRO-LAYOUT RATIFICATION (CP-1.0,
    //!    O-INT dispatch) — rule-9 block — 2026-08-16"
    //!    (reports/O-QA-log.md): the spec-silent stream-layout choices
    //!    the engine's hashes key are RATIFIED form (a) — these goldens
    //!    inherit that contract rather than pinning a new one.
    //! 2. The LIVE conformance test `caf_stream_conformance.rs` (this
    //!    crate; activated 2026-08-16, 12/12 double-derived vectors):
    //!    any stream drift trips it INDEPENDENTLY of these goldens, so
    //!    a golden mismatch here separates consumer-arm drift from
    //!    stream drift by construction.
    //!
    //! Spec: ES §45.3 (deterministic exemplar synthesis; failure rules
    //! 1/2), §40.5 (fill family); contracts content_aware_fill.yaml;
    //! D-48 (value-captured determinism; journal replay identity).
    //! Fixtures are fully deterministic: fixed envelopes, fixed ids,
    //! pinned seeds, no wall-clock/entropy anywhere.
    //!
    //! Stations (each = package + export png + meta, twice-produced and
    //! byte-compared by the regen bin; production re-proves undo/redo
    //! byte-identity and journal-replay identity as in-meta evidence):
    //! * `caf-heal-rect`      — red field, 12×12 blue blemish hole,
    //!   `output:"current_layer"`, seed 0, sampleArea auto.
    //! * `caf-heal-seed1`     — same fixture, seed 1 (seed sensitivity
    //!   recorded observationally in meta; determinism per seed is the
    //!   assertion).
    //! * `caf-params-rect-draft` — sampleArea rectangular, quality
    //!   draft, seed 7, colorAdaptation none.
    //! * `caf-starved-fallback` — §45.3 failure rule 1: near-all hole
    //!   (1-px sample ring) SUCCEEDS via the diffusion fallback.

    use crate::assembly_point;
    use appname_core::{export_png_bytes, save_package_bytes, Engine, IVec2};

    pub struct CafGolden {
        pub name: &'static str,
        pub package: Vec<u8>,
        pub export: Vec<u8>,
        /// Meta lines documenting fixed inputs + in-production evidence.
        pub meta: String,
    }

    fn env(n: u32, kind: &str, layer: Option<&str>, params: &str) -> String {
        let target = layer
            .map(|l| format!(r#","target":{{"layerId":"{l}"}}"#))
            .unwrap_or_default();
        format!(
            r#"{{"schema":"appname.command.v1","id":"caf90000-0000-4000-8000-0001{n:08x}","kind":"{kind}"{target},"params":{params}}}"#
        )
    }

    /// Red 96×64 field with a blue 12×12 blemish at (40,24); the
    /// blemish rect stays selected — it IS the hole H (§45.3).
    fn blemish_engine() -> (Engine, appname_core::LayerId, String) {
        let mut e = assembly_point::assembled_engine();
        e.apply_bytes(
            env(
                1,
                "doc.new",
                None,
                r#"{"width":96,"height":64,"background":"white"}"#,
            )
            .as_bytes(),
        )
        .expect("caf golden doc.new");
        let layer = e.document().unwrap().z_order_walk()[0];
        let l = layer.to_string();
        e.apply_json(&env(
            2,
            "fill",
            Some(&l),
            r#"{"content":"color","color":{"model":"rgb","comps":[1.0,0.0,0.0,1.0]}}"#,
        ))
        .expect("red field");
        e.apply_json(&env(
            3,
            "selection.set",
            None,
            r#"{"source":"rect","rect":{"x":40,"y":24,"w":12,"h":12},"antialias":false}"#,
        ))
        .expect("blemish select");
        e.apply_json(&env(
            4,
            "fill",
            Some(&l),
            r#"{"content":"color","color":{"model":"rgb","comps":[0.0,0.0,1.0,1.0]}}"#,
        ))
        .expect("blue blemish");
        (e, layer, l)
    }

    fn px(e: &Engine, layer: appname_core::LayerId, x: i32, y: i32) -> [f32; 4] {
        e.document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .read_pixel_f32(IVec2::new(x, y))
    }

    /// Apply the CAF envelope with full D-48 evidence: undo byte-exact,
    /// redo byte-exact, journal replay byte-exact. Returns the evidence
    /// meta lines.
    fn apply_caf_with_evidence(e: &mut Engine, caf_env: &str) -> String {
        let before = e.state_hash();
        e.apply_json(caf_env).expect("caf golden apply");
        let after = e.state_hash();
        e.undo().expect("caf golden undo");
        assert_eq!(e.state_hash(), before, "undo byte-identity");
        e.redo().expect("caf golden redo");
        assert_eq!(e.state_hash(), after, "redo byte-identity");
        let replayed = e.replay_assembled(e.journal()).expect("replay");
        assert_eq!(replayed.state_hash(), after, "journal replay identity");
        format!(
            "evidence: undo/redo byte-identity + journal-replay identity \
             re-proven at production (state hash {after:?})\n"
        )
    }

    pub fn produce_all() -> Vec<CafGolden> {
        let mut out = Vec::new();

        // 1. caf-heal-rect (seed 0) + 2. caf-heal-seed1
        let mut heal_exports: Vec<(u32, Vec<u8>)> = Vec::new();
        for (name, seed) in [("caf-heal-rect", 0u32), ("caf-heal-seed1", 1u32)] {
            let (mut e, layer, l) = blemish_engine();
            let evidence = apply_caf_with_evidence(
                &mut e,
                &env(
                    5,
                    "content_aware_fill",
                    Some(&l),
                    &format!(r#"{{"output":"current_layer","seed":{seed}}}"#),
                ),
            );
            let healed = px(&e, layer, 45, 30);
            assert!(
                healed[0] > 0.9 && healed[2] < 0.1,
                "{name}: hole synthesized from the red surround: {healed:?}"
            );
            let outside = px(&e, layer, 10, 10);
            assert!(outside[0] > 0.9, "{name}: outside the hole untouched");
            let export = export_png_bytes(&e).expect("caf export");
            heal_exports.push((seed, export.clone()));
            out.push(CafGolden {
                name,
                package: save_package_bytes(&e).expect("caf package"),
                export,
                meta: format!(
                    "inputs: doc.new 96x64 white; fill red; selection.set \
                     rect 40,24,12x12 aa=false; fill blue (blemish; rect \
                     stays selected = hole H); content_aware_fill \
                     output=current_layer seed={seed} sampleArea=auto \
                     (§45.3 pad rule)\n\
                     witnesses: hole center re-synthesized red \
                     (r>0.9, b<0.1 at 45,30); outside untouched at 10,10\n\
                     {evidence}"
                ),
            });
        }
        // Seed sensitivity: recorded observationally (not asserted — the
        // engine owes determinism PER seed, not divergence ACROSS seeds).
        let seeds_differ = heal_exports[0].1 != heal_exports[1].1;
        let note = format!(
            "observation: seed-0 and seed-1 exports {}\n",
            if seeds_differ { "differ" } else { "coincide" }
        );
        for g in out.iter_mut() {
            g.meta.push_str(&note);
        }

        // 3. caf-params-rect-draft
        {
            let (mut e, layer, l) = blemish_engine();
            let evidence = apply_caf_with_evidence(
                &mut e,
                &env(
                    5,
                    "content_aware_fill",
                    Some(&l),
                    r#"{"output":"current_layer","sampleArea":"rectangular","quality":"draft","seed":7,"colorAdaptation":"none"}"#,
                ),
            );
            let healed = px(&e, layer, 45, 30);
            assert!(
                healed[0] > 0.9 && healed[2] < 0.1,
                "params station healed: {healed:?}"
            );
            out.push(CafGolden {
                name: "caf-params-rect-draft",
                package: save_package_bytes(&e).expect("caf package"),
                export: export_png_bytes(&e).expect("caf export"),
                meta: format!(
                    "inputs: blemish fixture (as caf-heal-rect); \
                     content_aware_fill output=current_layer \
                     sampleArea=rectangular quality=draft seed=7 \
                     colorAdaptation=none\n\
                     witnesses: hole center re-synthesized red\n\
                     {evidence}"
                ),
            });
        }

        // 4. caf-starved-fallback (§45.3 failure rule 1)
        {
            let (mut e, layer, l) = blemish_engine();
            e.apply_json(&env(
                5,
                "selection.set",
                None,
                r#"{"source":"rect","rect":{"x":1,"y":1,"w":94,"h":62},"antialias":false}"#,
            ))
            .expect("starved hole select");
            let evidence = apply_caf_with_evidence(
                &mut e,
                &env(
                    6,
                    "content_aware_fill",
                    Some(&l),
                    r#"{"output":"current_layer","seed":0}"#,
                ),
            );
            let v = px(&e, layer, 48, 32);
            assert!(
                v.iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c)),
                "fallback pixels sane: {v:?}"
            );
            out.push(CafGolden {
                name: "caf-starved-fallback",
                package: save_package_bytes(&e).expect("caf package"),
                export: export_png_bytes(&e).expect("caf export"),
                meta: format!(
                    "inputs: blemish fixture; then selection.set rect \
                     1,1,94x62 aa=false (near-all hole, 1-px sample ring \
                     — fewer than N_min valid patch centers); \
                     content_aware_fill output=current_layer seed=0\n\
                     witnesses: §45.3 failure rule 1 — the command \
                     SUCCEEDS via the engine's diffusion fallback; \
                     pixels finite in [0,1]\n\
                     {evidence}"
                ),
            });
        }

        out
    }
}

/// `shasum -a 256`, the manifest checker the golden suites run.
///
/// W-19 (O-WIN): Windows has no `shasum`; Git for Windows' coreutils
/// `sha256sum` reads and checks the same `MANIFEST.sha256` format. Every
/// other platform runs exactly the command it ran before.
pub fn shasum256() -> std::process::Command {
    #[cfg(windows)]
    {
        std::process::Command::new("sha256sum")
    }
    #[cfg(not(windows))]
    {
        let mut c = std::process::Command::new("shasum");
        c.args(["-a", "256"]);
        c
    }
}

pub mod paths {
    //! Repo-relative path resolution for the harness.
    //!
    //! Spec: plan §2.1 repo layout (fixtures/, goldens/, contracts/ at the
    //! workspace root; this crate lives at tests/conformance).

    use std::path::PathBuf;

    pub fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("tests/conformance sits two levels under the repo root")
            .to_path_buf()
    }
    pub fn adversarial_dir() -> PathBuf {
        repo_root().join("fixtures").join("adversarial")
    }
    pub fn contracts_dir() -> PathBuf {
        repo_root().join("contracts")
    }
    pub fn workflows_dir() -> PathBuf {
        repo_root().join("tests").join("workflows")
    }
}

pub mod shipped_cli {
    //! Freshness-guarded resolution of the SHIPPED CLI binary (O-QA
    //! CP-1.0 ratification batch — the recorded backlog item: `cli_ni_sweep`
    //! trusted a prebuilt `target/debug/appname` with no staleness guard,
    //! so a binary predating a rebase could answer a superseded overlay
    //! table and red the suite for a non-reason; observed 2026-08-16,
    //! reports/O-QA-log.md CAF activation entry §3).
    //!
    //! Policy: rebuild UNCONDITIONALLY (`cargo build -p appname-cli`, a
    //! no-op when fresh) rather than trust an mtime comparison — mtimes
    //! lie across rebases/checkouts, and cargo's own dep tracking is the
    //! one freshness authority we already have. `APPNAME_CLI` stays the
    //! explicit override (the caller vouches for that binary; workflow
    //! scripts and CI wrappers use it deliberately). Once per test
    //! binary via `OnceLock`.

    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::OnceLock;

    static FRESH: OnceLock<PathBuf> = OnceLock::new();

    /// The shipped `appname` CLI, guaranteed to postdate the tree: either
    /// the caller's explicit `APPNAME_CLI` override, or
    /// `target/debug/appname` after an unconditional `cargo build
    /// -p appname-cli` (cheap when already fresh).
    pub fn fresh_cli_bin() -> PathBuf {
        FRESH
            .get_or_init(|| {
                if let Ok(p) = std::env::var("APPNAME_CLI") {
                    return PathBuf::from(p);
                }
                let root = crate::paths::repo_root();
                let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
                let out = Command::new(cargo)
                    .args(["build", "-p", "appname-cli"])
                    .current_dir(&root)
                    .output()
                    .expect("spawn `cargo build -p appname-cli` (freshness guard)");
                assert!(
                    out.status.success(),
                    "freshness guard: `cargo build -p appname-cli` failed:\n{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                // W-20 (O-WIN): the binary carries EXE_SUFFIX (".exe" on Windows, empty elsewhere).
                let p = root
                    .join("target")
                    .join("debug")
                    .join(format!("appname{}", std::env::consts::EXE_SUFFIX));
                assert!(
                    p.is_file(),
                    "{} missing after a successful build",
                    p.display()
                );
                p
            })
            .clone()
    }
}

pub mod corpus {
    //! Committed-corpus loading.
    //!
    //! Spec: fixtures/README.md case-record contract; plan §2.1 "Test
    //! harness v1" (corpus loading must run green at wave 1).

    use crate::json::{parse, Value};
    use crate::paths;
    use std::path::{Path, PathBuf};

    /// Every `.json` under `fixtures/adversarial/`, sorted by path
    /// (deterministic iteration, D-48 discipline).
    pub fn corpus_files() -> Vec<PathBuf> {
        let mut out = Vec::new();
        walk(&paths::adversarial_dir(), &mut out);
        out.sort();
        out
    }

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let rd =
            std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
        for entry in rd {
            let p = entry.expect("dir entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e == "json") {
                out.push(p);
            }
        }
    }

    pub fn load(path: &Path) -> Value {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        parse(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
    }

    pub fn manifest() -> Value {
        load(&paths::adversarial_dir().join("manifest.json"))
    }
    pub fn ported_envelope() -> Value {
        load(
            &paths::adversarial_dir()
                .join("ported")
                .join("envelope.json"),
        )
    }
    pub fn ported_cli() -> Value {
        load(&paths::adversarial_dir().join("ported").join("cli.json"))
    }
    pub fn envelope_rejects() -> Value {
        load(
            &paths::adversarial_dir()
                .join("generated")
                .join("envelope_rejects.json"),
        )
    }
    pub fn kind_file(kind: &str) -> Value {
        load(
            &paths::adversarial_dir()
                .join("generated")
                .join("kinds")
                .join(format!("{kind}.json")),
        )
    }
    /// Sorted kind names for which a `generated/kinds/<kind>.json` exists.
    pub fn kind_file_names() -> Vec<String> {
        let dir = paths::adversarial_dir().join("generated").join("kinds");
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
            .map(|e| e.expect("dir entry").path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .map(|p| p.file_stem().unwrap().to_str().unwrap().to_string())
            .collect();
        names.sort();
        names
    }
    pub fn cases(doc: &Value) -> &[Value] {
        doc.get("cases")
            .and_then(Value::as_arr)
            .expect("corpus document carries a cases array")
    }
}

pub mod contracts {
    //! Line-level readers for the two frozen contracts files the
    //! self-checks cross-validate against. Deliberately narrow: the mirror
    //! is verbatim-frozen (contract-sync gate), so the shapes below are
    //! stable; a full YAML reader is not warranted for a zero-dependency
    //! harness.
    //!
    //! Spec: contracts/commands/_index.yaml (appname.commandIndex.v1);
    //! contracts/message_ids.yaml (appname.messageid-registry.v1).

    use crate::paths;
    use std::collections::BTreeSet;

    /// The 102 command kinds, in `_index.yaml` row order.
    pub fn index_kinds() -> Vec<String> {
        let path = paths::contracts_dir().join("commands").join("_index.yaml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let mut kinds = Vec::new();
        for line in text.lines() {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix("- { kind: ") {
                let kind = rest
                    .split(',')
                    .next()
                    .expect("kind token before first comma")
                    .trim();
                kinds.push(kind.to_string());
            }
        }
        kinds
    }

    /// Registered `error.*` MessageIds (top-level `- id:` rows only;
    /// wildcard families are namespaces, not ids, and never appear as rows).
    pub fn v55_error_ids() -> BTreeSet<String> {
        let path = paths::contracts_dir().join("message_ids.yaml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let mut ids = BTreeSet::new();
        for line in text.lines() {
            // top-level list rows only: no leading indentation
            if let Some(rest) = line.strip_prefix("- id: ") {
                let id = rest.trim();
                if id.starts_with("error.") {
                    ids.insert(id.to_string());
                }
            }
        }
        ids
    }
}

pub mod expect {
    //! Expected-outcome record helpers shared by the self-checks and the
    //! (ignored) engine suites.
    //!
    //! Spec: fixtures/README.md case-record contract; ES §40.1 taxonomy.

    use crate::json::Value;

    /// Collect every error-code string referenced by expectation fields
    /// anywhere under `v` (cp01Expect.code, codeAnyOf entries,
    /// protoExpect.code, protoExpect.observedCode).
    pub fn collect_codes(v: &Value, out: &mut std::collections::BTreeSet<String>) {
        match v {
            Value::Obj(m) => {
                for (k, val) in m {
                    match (k.as_str(), val) {
                        ("code" | "observedCode", Value::Str(s)) => {
                            out.insert(s.clone());
                        }
                        ("codeAnyOf", Value::Arr(a)) => {
                            for e in a {
                                if let Value::Str(s) = e {
                                    out.insert(s.clone());
                                }
                            }
                        }
                        _ => collect_codes(val, out),
                    }
                }
            }
            Value::Arr(a) => {
                for e in a {
                    collect_codes(e, out);
                }
            }
            _ => {}
        }
    }

    /// Envelope-id shape: 8-4-4-4-12 lowercase hex, uuid version nibble 4.
    pub fn is_uuid_shaped(s: &str) -> bool {
        let b = s.as_bytes();
        if b.len() != 36 {
            return false;
        }
        for (i, c) in b.iter().enumerate() {
            match i {
                8 | 13 | 18 | 23 => {
                    if *c != b'-' {
                        return false;
                    }
                }
                14 => {
                    if *c != b'4' {
                        return false;
                    }
                }
                _ => {
                    if !c.is_ascii_hexdigit() || c.is_ascii_uppercase() {
                        return false;
                    }
                }
            }
        }
        true
    }

    #[cfg(test)]
    mod tests {
        //! Spec: uuid-shape helper checks.
        use super::is_uuid_shaped;

        #[test]
        fn uuid_shape() {
            assert!(is_uuid_shaped("a0000000-0000-4000-8000-000000000001"));
            assert!(!is_uuid_shaped("not-a-uuid"));
            assert!(!is_uuid_shaped("A0000000-0000-4000-8000-000000000001"));
            assert!(!is_uuid_shaped("a0000000-0000-1000-8000-000000000001"));
        }
    }
}

pub mod opnode_goldens {
    //! CP-0.7 OpNode re-render correctness goldens (checkpoint exit-gate
    //! row) — the re-render truth PRODUCIBLE on this main: the smart
    //! object's cached-composite SURFACE is rewritten from the swapped
    //! embedded source across `smartobject.edit_contents`, on EVERY
    //! instance, byte-exactly reversible across undo/redo.
    //!
    //! Spec: ES §11.5 (edit_contents swaps every instance; re-render),
    //! §12.1/§12.4 (chain evaluation, disabled == never-applied),
    //! §17.1/§11.4 (adjustment over backdrop); contracts
    //! smartobject.*.yaml / filter.smart_add.yaml / adjustment.add.yaml;
    //! plan §2.4 exit-gate row "OpNode re-render correctness goldens";
    //! plan §0.6 rule 6.
    //!
    //! SCOPE HISTORY: at the first CP-0.7 close these goldens were
    //! SCOPE-PINNED to the SURFACE level by finding QA-F8 (the composite
    //! doors refused/omitted smart layers), with a composite-door
    //! TRIPWIRE in the verify suite. The §12.1 row landed (core
    //! ChainEvaluator + export serving, 76d7d09; render
    //! `composite_document` serving, 55c3002), the tripwire FIRED as
    //! designed, and THIS revision executes its instructions: the
    //! goldens now pin the COMPOSITE level too — full-canvas composite
    //! bytes through the render door (`composite_document_with`, the
    //! §34.3 CPU oracle) at every station, plus two NEW stations for the
    //! §12.1 constructs that now composite (an enabled gaussian
    //! smart-filter chain and an unmasked invert adjustment layer).
    //! QA-F8 is CLOSED (lane-log goldens-round entry). The surface-level
    //! capture stays — it is the §11.5 cached-proxy truth the composite
    //! is built from, and the committed surface goldens did not move.

    use crate::assembly_point;
    use appname_core::geom::IRect;
    use appname_core::hash::ContentHash;
    use appname_core::Engine;

    pub const LID: &str = "c0000000-0000-4000-8000-00000000bbb1";
    pub const SOL: &str = "c0000000-0000-4000-8000-00000000bbb2";
    pub const DUP: &str = "c0000000-0000-4000-8000-00000000bbb3";
    pub const CONVERT_ASSET: &str = "ef89c10d-7f6b-5459-aeaa-ed4bc08828f4";
    /// The two §12.1 stations added at the composite extension (fixed
    /// creation ids, §9.6 client-generated).
    pub const CHAIN_OP: &str = "c0000000-0000-4000-8000-00000000ccc1";
    pub const ADJ_LAYER: &str = "c0000000-0000-4000-8000-00000000eee1";
    pub const ADJ_OP: &str = "c0000000-0000-4000-8000-00000000eee2";

    fn apply(engine: &mut Engine, raw: &str) {
        engine
            .apply_bytes(raw.as_bytes())
            .unwrap_or_else(|e| panic!("{raw} -> {:?} {}", e.code, e.message));
    }

    /// The smart layer's cached-composite surface as little-endian f32
    /// bytes over its bounds (row-major RGBA), plus the bounds rect.
    pub fn surface_bytes(engine: &Engine, layer: &str) -> (IRect, Vec<u8>) {
        let doc = engine.document().expect("doc");
        let id = doc
            .z_order_walk()
            .into_iter()
            .find(|l| l.as_uuid().to_string() == layer)
            .unwrap_or_else(|| panic!("layer {layer} present"));
        let so = doc
            .layer(id)
            .and_then(|n| n.smart_object())
            .expect("smart object");
        let buf = so.cached_composite();
        let b = buf.bounds();
        let mut out = Vec::new();
        for y in b.min.y..b.max.y {
            for x in b.min.x..b.max.x {
                for s in buf.read_pixel_f32(appname_core::IVec2::new(x, y)) {
                    out.extend_from_slice(&s.to_le_bytes());
                }
            }
        }
        (b, out)
    }

    /// Full-canvas composite through the RENDER door (the §34.3 CPU
    /// oracle, serving the engine's own §12.1 op registry) as
    /// little-endian f32 bytes (row-major RGBA over the canvas rect).
    /// This is the composite-level truth the fired tripwire demanded.
    pub fn composite_bytes(engine: &Engine) -> Vec<u8> {
        let doc = engine.document().expect("doc");
        let canvas = doc.geometry().canvas_rect();
        let comp = appname_render::composite_document_with(doc, engine.assembly(), canvas)
            .expect("composite door serves this flow (QA-F8 closed)");
        let mut out = Vec::new();
        for y in canvas.min.y..canvas.max.y {
            for x in canvas.min.x..canvas.max.x {
                for s in comp.pixel(appname_core::IVec2::new(x, y)) {
                    out.extend_from_slice(&s.to_le_bytes());
                }
            }
        }
        out
    }

    pub struct Station {
        pub name: &'static str,
        pub state_hash: String,
        /// (layer, surface content hash) for each live smart instance
        pub surfaces: Vec<(&'static str, String)>,
        /// content hash of the full-canvas composite (render door)
        pub composite: String,
    }

    pub struct Flow {
        pub stations: Vec<Station>,
        /// full surface bytes at the two content states (the visible
        /// re-render truth: filled-foreground vs blue fixture)
        pub post_convert_surface: Vec<u8>,
        pub post_edit_surface: Vec<u8>,
        /// full composite bytes at the four golden-pinned stations
        /// (composite-level extension per the fired tripwire)
        pub post_convert_composite: Vec<u8>,
        pub post_edit_composite: Vec<u8>,
        pub post_chain_composite: Vec<u8>,
        pub post_adjust_composite: Vec<u8>,
        pub blue_asset: String,
    }

    fn station(engine: &Engine, name: &'static str, dup_live: bool) -> Station {
        let mut surfaces = vec![(
            SOL,
            ContentHash::of_bytes(&surface_bytes(engine, SOL).1).to_hex(),
        )];
        if dup_live {
            surfaces.push((
                DUP,
                ContentHash::of_bytes(&surface_bytes(engine, DUP).1).to_hex(),
            ));
        }
        Station {
            name,
            state_hash: engine.state_hash().to_hex(),
            surfaces,
            composite: ContentHash::of_bytes(&composite_bytes(engine)).to_hex(),
        }
    }

    /// The deterministic flow (fixed envelope ids; the blue fixture is
    /// the committed fixtures/w16/blue-64.png, content-derived asset id
    /// asserted by the caller against the returned value).
    pub fn produce() -> Flow {
        let blue = std::fs::read(
            crate::paths::repo_root()
                .join("fixtures")
                .join("w16")
                .join("blue-64.png"),
        )
        .expect("fixtures/w16/blue-64.png");
        let mut e = assembly_point::assembled_engine();
        apply(
            &mut e,
            r#"{"schema":"appname.command.v1","id":"c07e0000-0000-4000-8000-000000000d0c","kind":"doc.new","params":{"width":64,"height":64,"background":"white"}}"#,
        );
        apply(
            &mut e,
            &format!(
                r#"{{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000aaa1","kind":"layer.add","params":{{"kind":"raster","newLayerId":"{LID}","name":"art"}}}}"#
            ),
        );
        apply(
            &mut e,
            &format!(
                r#"{{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000aaa2","kind":"fill","target":{{"layerId":"{LID}"}},"params":{{"content":"foreground"}}}}"#
            ),
        );
        apply(
            &mut e,
            &format!(
                r#"{{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000aaa3","kind":"smartobject.convert","params":{{"layerIds":["{LID}"],"newLayerId":"{SOL}"}}}}"#
            ),
        );
        let mut stations = vec![station(&e, "postConvert", false)];
        let post_convert_surface = surface_bytes(&e, SOL).1;
        let post_convert_composite = composite_bytes(&e);
        apply(
            &mut e,
            &format!(
                r#"{{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000aaa4","kind":"layer.duplicate","target":{{"layerId":"{SOL}"}},"params":{{"idMap":{{"{SOL}":"{DUP}"}}}}}}"#
            ),
        );
        stations.push(station(&e, "postDuplicate", true));
        let blue_asset = e.add_raster_asset(blue).expect("ingest blue").to_string();
        apply(
            &mut e,
            &format!(
                r#"{{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000aaa5","kind":"smartobject.edit_contents","target":{{"layerId":"{SOL}"}},"params":{{"assetBefore":"{CONVERT_ASSET}","assetAfter":"{blue_asset}"}}}}"#
            ),
        );
        stations.push(station(&e, "postEdit", true));
        let post_edit_surface = surface_bytes(&e, SOL).1;
        let post_edit_composite = composite_bytes(&e);
        e.undo().expect("undo edit_contents");
        stations.push(station(&e, "postUndo", true));
        e.redo().expect("redo edit_contents");
        stations.push(station(&e, "postRedo", true));

        // Composite-level extension stations (the fired tripwire's
        // instructions): the §12.1 constructs that now composite.
        // Station postChainAdd — enabled smart-filter chain on the DUP
        // instance (§12.1 per-tile evaluation over the proxy). Premise
        // choices, both observed at the first regeneration attempts and
        // recorded in the lane log: the chain rides the TOPMOST instance
        // (DUP — a chain on SOL is covered by the opaque DUP above and
        // composites to nothing), and the op is `filter.custom` with the
        // dyadic single-tap (+2,0) shift kernel (the render suite's
        // §16.3 precedent — a blur on the uniform blue proxy would be
        // invisible; the shift's edge band reads absent taps -> RGB 0
        // under intact alpha, a REAL deterministic composite delta).
        apply(
            &mut e,
            &format!(
                r#"{{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000aaa8","kind":"filter.smart_add","target":{{"layerId":"{DUP}"}},"params":{{"filterId":"filter.custom","params":{{"kernel":[[0,0,0,0,0],[0,0,0,0,0],[0,0,0,0,1],[0,0,0,0,0],[0,0,0,0,0]],"divisor":1,"offset":0.0}},"newOpId":"{CHAIN_OP}"}}}}"#
            ),
        );
        stations.push(station(&e, "postChainAdd", true));
        let post_chain_composite = composite_bytes(&e);
        // Station postAdjustAdd — unmasked invert adjustment layer at
        // the top of the stack (§17.1 n = 1 chain over the backdrop).
        apply(
            &mut e,
            &format!(
                r#"{{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000aaa9","kind":"adjustment.add","params":{{"opKind":"adjustment.invert","newLayerId":"{ADJ_LAYER}","newOpId":"{ADJ_OP}","params":{{}}}}}}"#
            ),
        );
        stations.push(station(&e, "postAdjustAdd", true));
        let post_adjust_composite = composite_bytes(&e);

        Flow {
            stations,
            post_convert_surface,
            post_edit_surface,
            post_convert_composite,
            post_edit_composite,
            post_chain_composite,
            post_adjust_composite,
            blue_asset,
        }
    }
}

pub mod retouch_goldens {
    //! Retouch-suite stroke goldens — the CP-1.0 retouch row's goldens,
    //! FROZEN by O-QA's golden round (the retouch row merged d660d3b with
    //! goldens STAGED-not-frozen; INTEGRATION-log 2026-08-18). Produced
    //! through the ASSEMBLED (production) engine, fixed seeds + fixed
    //! samples throughout — no wall-clock/entropy anywhere.
    //!
    //! Spec: contracts `clone.stroke.yaml` / `heal.stroke.yaml` /
    //! `dodge_burn.stroke.yaml` / `smudge.stroke.yaml` /
    //! `blur_sharpen.stroke.yaml`; ES §40.5 (stroke family), §21.1/§21.2
    //! (clone/heal), Part 45 §45.1 (heal = Poisson membrane commit; the
    //! merged appname-algo `poisson_heal` engine) / §45.2 / §45.3 (spot
    //! content-aware footprint = the deterministic exemplar engine — the
    //! CAF-consumer precedent); §15.4 (per-stroke seed); D-48 (value-
    //! captured determinism; journal replay identity). plan §0.6 rule 6;
    //! goldens/README.md rules 1–5.
    //!
    //! Determinism rationale, per family: dynamics RNG is command-seeded
    //! (PCG32, §15.4; omitted → 0, the recorded family default); Part 45
    //! retouch stochastic choices derive their seed from the envelope
    //! mergeKey (blake3, §45 intro; absent mergeKey → blake3 of the
    //! envelope id string) — replay is bit-identical either way, and the
    //! fixtures here pin fixed ids + fixed seeds so two independent
    //! productions agree byte-for-byte (rule 5). The oracle for each
    //! station is the in-production D-48 evidence: undo restores the
    //! pre-command state byte-exactly, redo restores the post-command
    //! state byte-exactly, and a fresh journal replay through the SAME
    //! assembled engine reproduces the final state hash (recorded in the
    //! `.meta`). Heal's SPOT arm rides §45.3 — its stream-layout
    //! contract is the same one the CAF goldens cite (the SEEDED-STREAM
    //! MICRO-LAYOUT RATIFICATION + the live `caf_stream_conformance`
    //! test), so a spot-heal drift trips that test INDEPENDENTLY of this
    //! golden.

    use crate::assembly_point;
    use crate::stroke_goldens::fixed_samples_basic;
    use appname_core::{export_png_bytes, save_package_bytes, ContentHash, Engine};

    pub struct RetouchGolden {
        pub name: &'static str,
        pub package: Vec<u8>,
        pub export: Vec<u8>,
        /// Meta lines: fixed inputs + in-production D-48 evidence.
        pub meta: String,
    }

    /// Fixed doc.new 128×96 white + samples asset; returns the engine,
    /// the background layer id string, and the samples asset uuid string.
    fn ready(id_tail: u32) -> (Engine, String, String) {
        let mut e = assembly_point::assembled_engine();
        let samples = e.add_samples_asset(fixed_samples_basic());
        let env = format!(
            r#"{{"schema":"appname.command.v1","id":"4e700000-0000-4000-8000-{id_tail:012}","kind":"doc.new","params":{{"width":128,"height":96,"background":"white"}}}}"#
        );
        e.apply_bytes(env.as_bytes()).expect("retouch doc.new");
        let layer = e.document().unwrap().z_order_walk()[0].to_string();
        (e, layer, samples.to_string())
    }

    /// A stroke envelope over the fixed samples path (§9.6 rule 2: the
    /// samples field is the assetId, never inline).
    fn stroke_env(id_tail: u32, kind: &str, layer: &str, samples: &str, params: &str) -> String {
        format!(
            r#"{{"schema":"appname.command.v1","id":"4e700000-0000-4000-8000-{id_tail:012}","kind":"{kind}","target":{{"layerId":"{layer}"}},"params":{params},"samples":"{samples}"}}"#
        )
    }

    /// Lay the fixed base content: a hard-round-16 red brush stroke along
    /// the samples path, so the retouch engines have non-uniform pixels to
    /// act on (dodge/smudge/blur on flat white would be near-no-ops).
    fn base_stroke(e: &mut Engine, id_tail: u32, layer: &str, samples: &str, color: &str) {
        let env = stroke_env(
            id_tail,
            "brush.stroke",
            layer,
            samples,
            &format!(
                r#"{{"brushPreset":"{}","color":{color},"seed":0,"opacity":1,"sizePx":10}}"#,
                assembly_point::preset_hard_round_16_id()
            ),
        );
        e.apply_bytes(env.as_bytes()).expect("retouch base stroke");
    }

    /// Apply the retouch envelope with full D-48 evidence: undo byte-
    /// exact, redo byte-exact, whole-journal replay byte-exact. Returns
    /// the evidence meta line + the post-apply state hash.
    fn apply_with_evidence(e: &mut Engine, env: &str) -> (String, ContentHash) {
        let before = e.state_hash();
        e.apply_json(env).expect("retouch golden apply");
        let after = e.state_hash();
        e.undo().expect("retouch golden undo");
        assert_eq!(e.state_hash(), before, "undo byte-identity");
        e.redo().expect("retouch golden redo");
        assert_eq!(e.state_hash(), after, "redo byte-identity");
        let replayed = e.replay_assembled(e.journal()).expect("replay");
        assert_eq!(replayed.state_hash(), after, "journal replay identity");
        (
            format!(
                "evidence: undo/redo byte-identity + journal-replay \
                 identity re-proven at production (state hash {after:?})\n"
            ),
            after,
        )
    }

    fn finish(e: &Engine, name: &'static str, inputs: &str, evidence: &str) -> RetouchGolden {
        RetouchGolden {
            name,
            package: save_package_bytes(e).expect("retouch package"),
            export: export_png_bytes(e).expect("retouch export"),
            meta: format!("{inputs}{evidence}"),
        }
    }

    pub fn produce_all() -> Vec<RetouchGolden> {
        let red = r#"{"model":"rgb","comps":[1,0,0,0]}"#;
        let dark = r#"{"model":"rgb","comps":[0,0,0,0]}"#;
        let mut out = Vec::new();

        // 1. clone.stroke (§21.1) — self-clone the red band with a −14 y
        //    offset; aligned default; seed 3.
        {
            let (mut e, layer, samples) = ready(0x000000000001);
            base_stroke(&mut e, 0x000000000002, &layer, &samples, red);
            let before = e.state_hash();
            let (evidence, _) = apply_with_evidence(
                &mut e,
                &stroke_env(
                    0x000000000003,
                    "clone.stroke",
                    &layer,
                    &samples,
                    &format!(
                        r#"{{"brushPreset":"{}","source":{{"layerId":"{layer}"}},"offset":[0,-14],"seed":3}}"#,
                        assembly_point::preset_hard_round_16_id()
                    ),
                ),
            );
            assert_ne!(before, e.state_hash(), "clone deposited pixels");
            out.push(finish(
                &e,
                "retouch-clone",
                "inputs: doc.new 128x96 white; brush.stroke hard-round-16 \
                 red[1,0,0,0] sizePx 10 seed 0 (base band); clone.stroke \
                 source=self offset [0,-14] aligned(default) seed 3 over \
                 the same samples path (§21.1 aligned-offset clone)\n\
                 witnesses: the clone deposits the offset source band \
                 (state hash changes)\n",
                &evidence,
            ));
        }

        // 2. heal.stroke — SAMPLED healing mode (§45.1 Poisson membrane
        //    commit; consumes the merged appname-algo poisson_heal
        //    engine). Dark blemish first, then heal from a +40 y offset.
        {
            let (mut e, layer, samples) = ready(0x000000000010);
            base_stroke(&mut e, 0x000000000011, &layer, &samples, dark);
            let before = e.state_hash();
            let (evidence, _) = apply_with_evidence(
                &mut e,
                &stroke_env(
                    0x000000000012,
                    "heal.stroke",
                    &layer,
                    &samples,
                    &format!(
                        r#"{{"brushPreset":"{}","mode":"healing","source":{{"layerId":"{layer}"}},"offset":[0,40],"sizePx":16}}"#,
                        assembly_point::preset_hard_round_16_id()
                    ),
                ),
            );
            assert_ne!(before, e.state_hash(), "heal changed pixels");
            out.push(finish(
                &e,
                "retouch-heal-sampled",
                "inputs: doc.new 128x96 white; brush.stroke hard-round-16 \
                 dark[0,0,0,0] sizePx 10 (blemish); heal.stroke \
                 mode=healing source=self offset [0,40] sizePx 16 — the \
                 §45.1 un-premultiplied per-channel Poisson membrane \
                 commit (appname-algo poisson_heal), alpha untouched\n\
                 witnesses: heal changes committed pixels (state hash \
                 changes); undo/redo/replay re-prove D-48 identity\n",
                &evidence,
            ));
        }

        // 3. heal.stroke — SPOT content-aware mode (§45.3 deterministic
        //    exemplar footprint; the CAF-consumer arm). No armed source;
        //    spotFill defaults content_aware.
        {
            let (mut e, layer, samples) = ready(0x000000000020);
            base_stroke(&mut e, 0x000000000021, &layer, &samples, dark);
            let (evidence, _) = apply_with_evidence(
                &mut e,
                &stroke_env(
                    0x000000000022,
                    "heal.stroke",
                    &layer,
                    &samples,
                    &format!(
                        r#"{{"brushPreset":"{}","mode":"spot","sizePx":12}}"#,
                        assembly_point::preset_hard_round_16_id()
                    ),
                ),
            );
            out.push(finish(
                &e,
                "retouch-heal-spot",
                "inputs: doc.new 128x96 white; brush.stroke hard-round-16 \
                 dark[0,0,0,0] sizePx 10 (blemish); heal.stroke mode=spot \
                 spotFill=content_aware(default) sizePx 12 — the §45.3 \
                 deterministic-exemplar footprint fill (auto-source). Its \
                 seeded-stream layout is the SAME contract the CAF goldens \
                 cite (SEEDED-STREAM MICRO-LAYOUT RATIFICATION 2026-08-16 \
                 + the live caf_stream_conformance test); a spot-heal \
                 stream drift trips that test independently of this \
                 golden\n\
                 witnesses: command SUCCEEDS via the exemplar engine; \
                 undo/redo/replay re-prove D-48 identity\n",
                &evidence,
            ));
        }

        // 4. dodge_burn.stroke — tonal-range weighted (§21.5). Dodge then
        //    burn, both midtones, exposure 1.0 (boundary max), over the
        //    red band.
        {
            let (mut e, layer, samples) = ready(0x000000000030);
            base_stroke(&mut e, 0x000000000031, &layer, &samples, red);
            let preset = assembly_point::preset_hard_round_16_id();
            e.apply_json(&stroke_env(
                0x000000000032,
                "dodge_burn.stroke",
                &layer,
                &samples,
                &format!(
                    r#"{{"brushPreset":"{preset}","mode":"dodge","range":"midtones","exposure":1.0}}"#
                ),
            ))
            .expect("dodge");
            let (evidence, _) = apply_with_evidence(
                &mut e,
                &stroke_env(
                    0x000000000033,
                    "dodge_burn.stroke",
                    &layer,
                    &samples,
                    &format!(
                        r#"{{"brushPreset":"{preset}","mode":"burn","range":"midtones","exposure":1.0}}"#
                    ),
                ),
            );
            out.push(finish(
                &e,
                "retouch-dodge-burn",
                "inputs: doc.new 128x96 white; brush.stroke hard-round-16 \
                 red[1,0,0,0] sizePx 10 (base band); dodge.stroke \
                 mode=dodge range=midtones exposure 1.0; then \
                 dodge_burn.stroke mode=burn range=midtones exposure 1.0 \
                 — §21.5 tonal-range-weighted dodge/burn over the band\n\
                 witnesses: both tonal commands journaled; undo/redo/replay \
                 re-prove D-48 identity around the final (burn) command\n",
                &evidence,
            ));
        }

        // 5. smudge.stroke — libmypaint-memo pull semantics; strength 0.8.
        {
            let (mut e, layer, samples) = ready(0x000000000040);
            base_stroke(&mut e, 0x000000000041, &layer, &samples, red);
            let (evidence, _) = apply_with_evidence(
                &mut e,
                &stroke_env(
                    0x000000000042,
                    "smudge.stroke",
                    &layer,
                    &samples,
                    &format!(
                        r#"{{"brushPreset":"{}","strength":0.8}}"#,
                        assembly_point::preset_hard_round_16_id()
                    ),
                ),
            );
            out.push(finish(
                &e,
                "retouch-smudge",
                "inputs: doc.new 128x96 white; brush.stroke hard-round-16 \
                 red[1,0,0,0] sizePx 10 (base band); smudge.stroke \
                 strength 0.8 over the same samples path (drags color \
                 along the stroke)\n\
                 witnesses: smudge journaled; undo/redo/replay re-prove \
                 D-48 identity\n",
                &evidence,
            ));
        }

        // 6. blur_sharpen.stroke — focus tools. Blur then sharpen (the
        //    sharpen carries protectDetail, legal only with mode:sharpen).
        {
            let (mut e, layer, samples) = ready(0x000000000050);
            base_stroke(&mut e, 0x000000000051, &layer, &samples, red);
            let preset = assembly_point::preset_hard_round_16_id();
            e.apply_json(&stroke_env(
                0x000000000052,
                "blur_sharpen.stroke",
                &layer,
                &samples,
                &format!(r#"{{"brushPreset":"{preset}","mode":"blur","strength":1.0}}"#),
            ))
            .expect("blur");
            let (evidence, _) = apply_with_evidence(
                &mut e,
                &stroke_env(
                    0x000000000053,
                    "blur_sharpen.stroke",
                    &layer,
                    &samples,
                    &format!(
                        r#"{{"brushPreset":"{preset}","mode":"sharpen","strength":1.0,"protectDetail":true}}"#
                    ),
                ),
            );
            out.push(finish(
                &e,
                "retouch-blur-sharpen",
                "inputs: doc.new 128x96 white; brush.stroke hard-round-16 \
                 red[1,0,0,0] sizePx 10 (base band); blur_sharpen.stroke \
                 mode=blur strength 1.0; then blur_sharpen.stroke \
                 mode=sharpen strength 1.0 protectDetail true — the focus \
                 tools over the band\n\
                 witnesses: both focus commands journaled; undo/redo/replay \
                 re-prove D-48 identity around the final (sharpen) \
                 command\n",
                &evidence,
            ));
        }

        out
    }
}

pub mod composite_goldens {
    //! Composite SERVE-goldens — text-layer and shape-layer rendered
    //! composites, FROZEN by O-QA's golden round now that both composite
    //! walls fell (INTEGRATION-log 2026-08-18 @ 3b21d8f: "both composite
    //! walls FALL … export and readback now SERVE identically"). Produced
    //! through the ASSEMBLED (production) engine.
    //!
    //! Spec: ES §34.3 (the CPU reference composite is the parity oracle),
    //! §12.1 (parametric re-render arm), §20.2/§13.1 (shape coverage ×
    //! fill), §23 (text layout → page); contracts `vector.shape_add.yaml`
    //! / `text.add.yaml`; plan §0.6 rule 6; goldens/README.md rules.
    //!
    //! THE GOLDEN'S OWN INVARIANT (the wall that fell): for each station,
    //! the PNG export equals the encoded f32 readback plane BYTE-FOR-BYTE
    //! — `export_png_bytes(e)` == `encode_composite_png(composite_plane_f32
    //! (e))`. Both arms run the SAME §34.3 composite math (the readback
    //! door is literally `export_png_bytes` minus the final encode), so
    //! this equality is the "one-truth" wall's standing proof; the regen
    //! asserts it before writing, and the verify suite re-asserts it on a
    //! fresh assembled production.
    //!
    //! FILL-MATH POSTURE (pinned vs flagged): SOLID fills SERVE and are
    //! frozen here (shape solid fill-math; text pages carry their own
    //! solid fill). GRADIENT and PATTERN fills are the recorded §22 gap —
    //! they answer a typed refusal at the fill seam and are NOT frozen
    //! this round (flagged, deferral recorded below and in the lane log).
    //! Text content is the deterministic §23.5/§44.2 TOFU rendering (hex
    //! boxes) — no font is vendored (§3.10); the tofu geometry is fixed,
    //! so the composite is fully reproducible without a provided face.
    //! When a procedural face lands as a committed fixture, a face-bearing
    //! text station joins here (deferral recorded).

    use crate::assembly_point;
    use appname_core::{
        composite_plane_f32, encode_composite_png, export_png_bytes, save_package_bytes, Engine,
    };

    pub struct CompositeGolden {
        pub name: &'static str,
        pub package: Vec<u8>,
        pub export: Vec<u8>,
        pub meta: String,
    }

    fn apply(e: &mut Engine, env: &str, what: &str) {
        e.apply_json(env)
            .unwrap_or_else(|err| panic!("composite golden {what}: {err:?}"));
    }

    /// Apply the FINAL creation command with D-48 evidence (undo/redo/
    /// replay byte-identity), then assert the export == encoded-readback
    /// wall invariant. Returns the evidence + invariant meta lines.
    fn apply_and_prove(e: &mut Engine, env: &str, what: &str) -> String {
        let before = e.state_hash();
        apply(e, env, what);
        let after = e.state_hash();
        e.undo().expect("composite golden undo");
        assert_eq!(e.state_hash(), before, "undo byte-identity");
        e.redo().expect("composite golden redo");
        assert_eq!(e.state_hash(), after, "redo byte-identity");
        let replayed = e.replay_assembled(e.journal()).expect("replay");
        assert_eq!(replayed.state_hash(), after, "journal replay identity");
        // THE WALL: export bytes == encoded f32 readback bytes.
        let export = export_png_bytes(e).expect("composite export serves");
        let (w, h, plane) = composite_plane_f32(e).expect("composite readback serves");
        let readback = encode_composite_png(w, h, &plane).expect("encode readback");
        assert_eq!(
            export, readback,
            "{what}: export != readback (the one-truth wall must hold)"
        );
        format!(
            "evidence: undo/redo byte-identity + journal-replay identity \
             re-proven at production (state hash {:?})\n\
             invariant: export_png_bytes == encode(composite_plane_f32) \
             byte-for-byte ({} bytes) — the export==readback one-truth \
             wall (§34.3 single composite math)\n",
            after,
            export.len(),
        )
    }

    pub fn produce_all() -> Vec<CompositeGolden> {
        let mut out = Vec::new();

        // 1. composite-shape — two fixed vector paths (ellipse + rect),
        //    both SOLID fills, zero-width stroke (fill-only v1). Real §20
        //    geometry × the fill-math seam, served through the production
        //    assembly.
        {
            let mut e = assembly_point::assembled_engine();
            apply(
                &mut e,
                r#"{"schema":"appname.command.v1","id":"c05e0000-0000-4000-8000-000000000001","kind":"doc.new","params":{"width":48,"height":48,"background":"white","depth":"f32"}}"#,
                "doc.new",
            );
            apply(
                &mut e,
                r#"{"schema":"appname.command.v1","id":"c05e0000-0000-4000-8000-000000000002","kind":"vector.shape_add","params":{"primitive":{"kind":"ellipse","geometry":{"x":6,"y":6,"w":24,"h":24}},"fill":{"type":"solid","color":{"model":"rgba","comps":[0,0.5,1,1]}},"stroke":{"widthPx":0},"newLayerId":"c05e0000-0000-4000-8000-0000000000e1"}}"#,
                "shape ellipse",
            );
            let evidence = apply_and_prove(
                &mut e,
                r#"{"schema":"appname.command.v1","id":"c05e0000-0000-4000-8000-000000000003","kind":"vector.shape_add","params":{"primitive":{"kind":"rect","geometry":{"x":22,"y":22,"w":20,"h":20}},"fill":{"type":"solid","color":{"model":"rgba","comps":[1,0.4,0,1]}},"stroke":{"widthPx":0},"newLayerId":"c05e0000-0000-4000-8000-0000000000e2"}}"#,
                "shape rect",
            );
            out.push(CompositeGolden {
                name: "composite-shape",
                package: save_package_bytes(&e).expect("shape package"),
                export: export_png_bytes(&e).expect("shape export"),
                meta: format!(
                    "inputs: doc.new 48x48 white depth=f32; vector.shape_add \
                     ellipse [6,6,24x24] solid rgba[0,0.5,1,1] stroke 0; \
                     vector.shape_add rect [22,22,20x20] solid rgba[1,0.4,0,1] \
                     stroke 0 — real §20 geometry × the fill-math seam \
                     (coverage × premultiplied solid fill)\n\
                     fill-math: SOLID fills served + frozen; gradient/pattern \
                     are the §22 gap (typed refusal, NOT frozen this round)\n\
                     witnesses: shape composite SERVES (was the QA-F8 typed \
                     wall before the fill-seam row)\n\
                     {evidence}"
                ),
            });
        }

        // 2. composite-text — a point-text layer composited through the
        //    Door-6 layout→page arm. Deterministic §23.5/§44.2 TOFU
        //    rendering (no vendored face); solid black fill.
        {
            let mut e = assembly_point::assembled_engine();
            apply(
                &mut e,
                r#"{"schema":"appname.command.v1","id":"c07e0000-0000-4000-8000-000000000001","kind":"doc.new","params":{"width":128,"height":48,"background":"white","depth":"f32"}}"#,
                "doc.new",
            );
            let evidence = apply_and_prove(
                &mut e,
                r#"{"schema":"appname.command.v1","id":"c07e0000-0000-4000-8000-000000000002","kind":"text.add","params":{"layout":"point","origin":[8,16],"text":"Appname","style":{"size":24,"fill":{"type":"solid","color":{"model":"rgba","comps":[0,0,0,1]}}},"newLayerId":"c07e0000-0000-4000-8000-0000000000f1"}}"#,
                "text.add",
            );
            out.push(CompositeGolden {
                name: "composite-text",
                package: save_package_bytes(&e).expect("text package"),
                export: export_png_bytes(&e).expect("text export"),
                meta: format!(
                    "inputs: doc.new 128x48 white depth=f32; text.add \
                     layout=point origin [8,16] text \"Appname\" size 24 \
                     solid rgba[0,0,0,1] — composed through the Door-6 \
                     layout_and_compose page arm (D-48 page value-capture)\n\
                     content: deterministic §23.5/§44.2 TOFU hex boxes (NO \
                     font vendored, §3.10); fixed geometry = reproducible \
                     without a provided face; a face-bearing station is \
                     deferred until a procedural face lands as a committed \
                     fixture (recorded)\n\
                     witnesses: text composite SERVES (frame-16 tofu wall \
                     fell with the composite legs)\n\
                     {evidence}"
                ),
            });
        }

        out
    }
}
