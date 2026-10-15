// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `plugins-state.toml` — the `appname.pluginstate.v1` enable/grant
//! persistence (ES §25.6 [NORMATIVE]).
//!
//! Format note (deliberate, lane-log recorded): the spec pins TOML for
//! this file. No `toml` crate has a `deps/` record, so this module
//! carries a MINIMAL, STRICT reader/writer for exactly the
//! `appname.pluginstate.v1` shape — top-level `schema = "…"`,
//! `[[plugin]]` tables, string / boolean / string-array values, `#`
//! comments, no nesting, no dotted keys, no multi-line strings.
//! Anything outside the subset REFUSES TYPED (this file is app-owned
//! state, but a hand-edited or hostile file must fail loudly, never
//! misparse silently — W08 discipline). If O-BUILD later records a TOML
//! crate, the swap is contained here.
//!
//! Write side is deterministic: sorted plugin ids, one canonical
//! spelling, trailing newline; the round-trip test pins byte identity.

use std::fmt;
use std::path::Path;

use crate::plugin::permissions::{Grant, GrantSet};

/// One `[[plugin]]` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginState {
    pub id: String,
    pub version: String,
    pub enabled: bool,
    pub granted: GrantSet,
    /// Base64 as-persisted (pinned at install; key CHANGES re-prompt,
    /// §25.5 — comparison is the lifecycle's job). Empty = unsigned.
    pub public_key: String,
}

/// The whole file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateFile {
    pub plugins: Vec<PluginState>,
}

/// Typed state-file refusals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    Io(String),
    /// Line number + what refused.
    Malformed(usize, String),
    WrongSchema,
    /// A grant string that does not parse (line number carried).
    BadGrant(usize, String),
    DuplicatePlugin(String),
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "plugins-state: {e}"),
            Self::Malformed(n, m) => write!(f, "plugins-state line {n}: {m}"),
            Self::WrongSchema => write!(f, "plugins-state schema is not appname.pluginstate.v1"),
            Self::BadGrant(n, g) => write!(f, "plugins-state line {n}: bad grant {g:?}"),
            Self::DuplicatePlugin(id) => write!(f, "plugins-state: duplicate plugin {id}"),
        }
    }
}

impl std::error::Error for StateError {}

/// Unescape a TOML basic string body (the subset: `\"`, `\\`, `\n`,
/// `\t`; anything else refuses).
fn unquote(line_no: usize, s: &str) -> Result<String, StateError> {
    let inner = s
        .strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .ok_or_else(|| {
            StateError::Malformed(line_no, format!("expected a quoted string, got {s}"))
        })?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '"' {
            // An unescaped quote inside the body means the strip above
            // matched a broader span than one string — refuse.
            return Err(StateError::Malformed(
                line_no,
                "stray quote in string".into(),
            ));
        }
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            other => {
                return Err(StateError::Malformed(
                    line_no,
                    format!(
                        "unsupported escape \\{}",
                        other.map(String::from).unwrap_or_default()
                    ),
                ))
            }
        }
    }
    Ok(out)
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Split a `["a", "b"]` array body into element strings (subset: quoted
/// strings only, comma-separated).
fn parse_string_array(line_no: usize, s: &str) -> Result<Vec<String>, StateError> {
    let inner = s
        .strip_prefix('[')
        .and_then(|x| x.strip_suffix(']'))
        .ok_or_else(|| StateError::Malformed(line_no, format!("expected an array, got {s}")))?
        .trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    // Elements are quoted strings; a comma inside quotes is part of the
    // element. Walk with a tiny in-quotes state machine.
    let mut elems = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for c in inner.chars() {
        if in_quotes {
            current.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_quotes = false;
            }
            continue;
        }
        match c {
            '"' => {
                current.push(c);
                in_quotes = true;
            }
            ',' => {
                let e = current.trim().to_string();
                if e.is_empty() {
                    return Err(StateError::Malformed(line_no, "empty array element".into()));
                }
                elems.push(e);
                current = String::new();
            }
            c if c.is_whitespace() => {}
            other => {
                return Err(StateError::Malformed(
                    line_no,
                    format!("unexpected {other:?} in array"),
                ))
            }
        }
    }
    if in_quotes {
        return Err(StateError::Malformed(
            line_no,
            "unterminated string in array".into(),
        ));
    }
    let tail = current.trim().to_string();
    if !tail.is_empty() {
        elems.push(tail);
    }
    elems.into_iter().map(|e| unquote(line_no, &e)).collect()
}

impl StateFile {
    /// Parse the strict subset (module docs).
    pub fn parse(text: &str) -> Result<Self, StateError> {
        let mut schema_seen = false;
        let mut plugins: Vec<PluginState> = Vec::new();
        // Fields of the in-progress [[plugin]] table.
        struct Partial {
            line: usize,
            id: Option<String>,
            version: Option<String>,
            enabled: Option<bool>,
            granted: Option<Vec<String>>,
            public_key: Option<String>,
        }
        let mut current: Option<Partial> = None;

        let finish = |p: Partial, plugins: &mut Vec<PluginState>| -> Result<(), StateError> {
            let id = p
                .id
                .ok_or_else(|| StateError::Malformed(p.line, "plugin table without id".into()))?;
            if plugins.iter().any(|x| x.id == id) {
                return Err(StateError::DuplicatePlugin(id));
            }
            let mut granted = GrantSet::new();
            for (i, g) in p.granted.unwrap_or_default().iter().enumerate() {
                let grant =
                    Grant::parse(g).ok_or_else(|| StateError::BadGrant(p.line + i, g.clone()))?;
                granted.insert(grant);
            }
            plugins.push(PluginState {
                id,
                version: p.version.unwrap_or_default(),
                enabled: p.enabled.unwrap_or(false),
                granted,
                public_key: p.public_key.unwrap_or_default(),
            });
            Ok(())
        };

        for (idx, raw) in text.lines().enumerate() {
            let line_no = idx + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line == "[[plugin]]" {
                if let Some(p) = current.take() {
                    finish(p, &mut plugins)?;
                }
                current = Some(Partial {
                    line: line_no,
                    id: None,
                    version: None,
                    enabled: None,
                    granted: None,
                    public_key: None,
                });
                continue;
            }
            if line.starts_with('[') {
                return Err(StateError::Malformed(
                    line_no,
                    format!("unknown table {line}"),
                ));
            }
            let (key, value) = line
                .split_once('=')
                .map(|(k, v)| (k.trim(), v.trim()))
                .ok_or_else(|| {
                    StateError::Malformed(line_no, format!("not a key = value line: {line}"))
                })?;
            match (&mut current, key) {
                (None, "schema") => {
                    if unquote(line_no, value)? != "appname.pluginstate.v1" {
                        return Err(StateError::WrongSchema);
                    }
                    schema_seen = true;
                }
                (None, other) => {
                    return Err(StateError::Malformed(
                        line_no,
                        format!("unknown top-level key {other}"),
                    ))
                }
                (Some(p), "id") => p.id = Some(unquote(line_no, value)?),
                (Some(p), "version") => p.version = Some(unquote(line_no, value)?),
                (Some(p), "enabled") => {
                    p.enabled = Some(match value {
                        "true" => true,
                        "false" => false,
                        _ => {
                            return Err(StateError::Malformed(
                                line_no,
                                format!("enabled must be true|false, got {value}"),
                            ))
                        }
                    })
                }
                (Some(p), "granted") => p.granted = Some(parse_string_array(line_no, value)?),
                (Some(p), "public_key") => p.public_key = Some(unquote(line_no, value)?),
                (Some(_), other) => {
                    return Err(StateError::Malformed(
                        line_no,
                        format!("unknown plugin key {other}"),
                    ))
                }
            }
        }
        if let Some(p) = current.take() {
            finish(p, &mut plugins)?;
        }
        if !schema_seen {
            return Err(StateError::WrongSchema);
        }
        Ok(Self { plugins })
    }

    /// Canonical serialization (module docs: deterministic).
    pub fn to_toml(&self) -> String {
        let mut plugins: Vec<&PluginState> = self.plugins.iter().collect();
        plugins.sort_by(|a, b| a.id.cmp(&b.id));
        let mut out = String::new();
        out.push_str("# appname plugins-state (generated; appname.pluginstate.v1, ES 25.6)\n");
        out.push_str("schema = \"appname.pluginstate.v1\"\n");
        for p in plugins {
            out.push_str("\n[[plugin]]\n");
            out.push_str(&format!("id = {}\n", quote(&p.id)));
            out.push_str(&format!("version = {}\n", quote(&p.version)));
            out.push_str(&format!("enabled = {}\n", p.enabled));
            let grants: Vec<String> = p
                .granted
                .iter()
                .map(|g| quote(&g.to_grant_string()))
                .collect();
            out.push_str(&format!("granted = [{}]\n", grants.join(", ")));
            out.push_str(&format!("public_key = {}\n", quote(&p.public_key)));
        }
        out
    }

    pub fn load(path: &Path) -> Result<Self, StateError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(StateError::Io(format!("{}: {e}", path.display()))),
        }
    }

    /// Atomic save (temp + rename, the §7 package discipline).
    pub fn save(&self, path: &Path) -> Result<(), StateError> {
        let io = |e: std::io::Error| StateError::Io(format!("{}: {e}", path.display()));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, self.to_toml()).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&PluginState> {
        self.plugins.iter().find(|p| p.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut PluginState> {
        self.plugins.iter_mut().find(|p| p.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[cfg(not(windows))]
    fn spec_example() -> &'static str {
        // The ES §25.6 example verbatim (minus the trailing comment
        // form differences the subset keeps anyway).
        r#"schema = "appname.pluginstate.v1"
[[plugin]]
id = "org.example.blurpack"
version = "1.2.0"
enabled = true
granted = ["filter.register", "fs.read:/home/a/textures"]
public_key = "base64s"
"#
    }

    /// W-22: the same example with a Windows grant directory (TOML
    /// basic-string escaping doubles the backslashes).
    #[cfg(windows)]
    fn spec_example() -> &'static str {
        r#"schema = "appname.pluginstate.v1"
[[plugin]]
id = "org.example.blurpack"
version = "1.2.0"
enabled = true
granted = ["filter.register", "fs.read:C:\\Users\\a\\textures"]
public_key = "base64s"
"#
    }

    #[cfg(not(windows))]
    const SPEC_TEXTURES: &str = "/home/a/textures";
    #[cfg(windows)]
    const SPEC_TEXTURES: &str = r"C:\Users\a\textures";

    #[test]
    fn the_spec_example_parses() {
        let f = StateFile::parse(spec_example()).unwrap();
        assert_eq!(f.plugins.len(), 1);
        let p = &f.plugins[0];
        assert_eq!(p.id, "org.example.blurpack");
        assert!(p.enabled);
        assert_eq!(p.granted.iter().count(), 2);
        assert!(p
            .granted
            .fs_read_dirs()
            .any(|d| d == &PathBuf::from(SPEC_TEXTURES)));
    }

    #[test]
    fn round_trip_is_byte_stable() {
        let f = StateFile::parse(spec_example()).unwrap();
        let once = f.to_toml();
        let again = StateFile::parse(&once).unwrap().to_toml();
        assert_eq!(once, again);
    }

    #[test]
    fn hostile_state_refuses_typed() {
        // wrong schema
        assert!(matches!(
            StateFile::parse("schema = \"appname.pluginstate.v2\"\n"),
            Err(StateError::WrongSchema)
        ));
        // no schema at all
        assert!(matches!(StateFile::parse(""), Err(StateError::WrongSchema)));
        // unknown key
        assert!(matches!(
            StateFile::parse("schema = \"appname.pluginstate.v1\"\nrootkit = true\n"),
            Err(StateError::Malformed(2, _))
        ));
        // bad grant string
        let bad = "schema = \"appname.pluginstate.v1\"\n[[plugin]]\nid = \"a.b\"\ngranted = [\"fs.read:relative\"]\n";
        assert!(matches!(
            StateFile::parse(bad),
            Err(StateError::BadGrant(..))
        ));
        // duplicate plugin id
        let dup = "schema = \"appname.pluginstate.v1\"\n[[plugin]]\nid = \"a.b\"\n[[plugin]]\nid = \"a.b\"\n";
        assert!(matches!(
            StateFile::parse(dup),
            Err(StateError::DuplicatePlugin(_))
        ));
        // nesting outside the subset
        assert!(StateFile::parse("schema = \"appname.pluginstate.v1\"\n[table]\n").is_err());
    }

    #[test]
    fn save_load_round_trip() {
        let dir =
            std::env::temp_dir().join(format!("appname-state-{}", uuid::Uuid::new_v4().simple()));
        let path = dir.join("plugins-state.toml");
        let f = StateFile::parse(spec_example()).unwrap();
        f.save(&path).unwrap();
        let g = StateFile::load(&path).unwrap();
        assert_eq!(f, g);
        // Missing file = empty default, not an error (first run).
        let empty = StateFile::load(&dir.join("absent.toml")).unwrap();
        assert!(empty.plugins.is_empty());
    }

    #[test]
    fn quoting_survives_awkward_strings() {
        let mut f = StateFile::default();
        f.plugins.push(PluginState {
            id: "a.b".into(),
            version: "1.0.0+meta\"x\\y".into(),
            enabled: false,
            granted: GrantSet::new(),
            public_key: String::new(),
        });
        let text = f.to_toml();
        let back = StateFile::parse(&text).unwrap();
        assert_eq!(back.plugins[0].version, "1.0.0+meta\"x\\y");
    }
}
