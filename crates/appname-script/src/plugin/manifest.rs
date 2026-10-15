// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The `appname.plugin.v1` manifest — parse and STRICT validation.
//!
//! Spec: ES §25.2 [NORMATIVE] carries the JSON Schema verbatim (the
//! schema is spec-inline; there is no `contracts/schemas/plugin.v1.json`
//! mirror — divergence-watch note in the lane log). This is a hand
//! validator in the house style (the filter-registry strictness-sweep
//! precedent): typed errors, unknown-key refusal, no silent tolerance.
//!
//! Validation posture: a manifest is UNTRUSTED INPUT from an untrusted
//! package. Every check refuses typed ([`ManifestError`]); nothing is
//! defaulted that the schema marks required.

use std::fmt;

use serde_json::Value;

use crate::plugin::permissions::Capability;

/// One `provides` entry: `{type, id}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provides {
    pub kind: ProvidedKind,
    pub id: String,
}

/// The closed `provides[].type` enum (ES §25.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvidedKind {
    Filter,
    Tool,
    Codec,
    Panel,
    Action,
    Adjustment,
}

impl ProvidedKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "filter" => Self::Filter,
            "tool" => Self::Tool,
            "codec" => Self::Codec,
            "panel" => Self::Panel,
            "action" => Self::Action,
            "adjustment" => Self::Adjustment,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Filter => "filter",
            Self::Tool => "tool",
            Self::Codec => "codec",
            Self::Panel => "panel",
            Self::Action => "action",
            Self::Adjustment => "adjustment",
        }
    }

    /// The registration capability this provider kind requires in the
    /// manifest `permissions` array (§25.3 capability table row 1;
    /// `action` rides `menu.action`, `adjustment` rides
    /// `filter.register` — the §17 adjustment registry is served by the
    /// filter trampoline, divergence-watch note in the lane log).
    pub fn required_capability(self) -> Capability {
        match self {
            Self::Filter | Self::Adjustment => Capability::FilterRegister,
            Self::Tool => Capability::ToolRegister,
            Self::Codec => Capability::CodecRegister,
            Self::Panel => Self::panel_cap(),
            Self::Action => Capability::MenuAction,
        }
    }

    fn panel_cap() -> Capability {
        Capability::PanelRegister
    }
}

/// The ABI range the plugin targets: `"abi_major.abi_minor"` strings,
/// `max` may be `"N.x"` (any minor of major N — the §25.6 repository
/// index uses exactly that form).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiBound {
    pub major: u32,
    /// `None` = `x` (any minor; only legal on `max`).
    pub minor: Option<u32>,
}

impl AbiBound {
    fn parse(s: &str, wildcard_ok: bool) -> Option<Self> {
        let (maj, min) = s.split_once('.')?;
        let major: u32 = parse_plain_u32(maj)?;
        if min == "x" {
            if !wildcard_ok {
                return None;
            }
            return Some(Self { major, minor: None });
        }
        let minor = parse_plain_u32(min)?;
        Some(Self {
            major,
            minor: Some(minor),
        })
    }
}

fn parse_plain_u32(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 9 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// A parsed, validated `appname.plugin.v1` manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Reverse-DNS id, `^[a-z0-9.]+$`.
    pub id: String,
    pub version: String,
    pub api_min: AbiBound,
    pub api_max: AbiBound,
    /// `plugin.wasm` | `plugin.lua` | `module.py` — a bare filename
    /// inside the package (path separators and traversal REFUSED).
    pub entry: String,
    pub permissions: Vec<Capability>,
    pub provides: Vec<Provides>,
    pub license: Option<String>,
    pub author: Option<String>,
}

/// The entry kind, derived from the manifest `entry` extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// `*.wasm` — the wasmtime host.
    Wasm,
    /// `*.lua` — the mlua host.
    Lua,
    /// `*.py` — the TRUSTED tier (not this module's host; lifecycle
    /// still models it so the Plugin Manager lists every tier).
    Python,
}

impl Manifest {
    /// Which sandbox host the entry names.
    pub fn entry_kind(&self) -> EntryKind {
        // Validation guaranteed one of the three suffixes.
        if self.entry.ends_with(".wasm") {
            EntryKind::Wasm
        } else if self.entry.ends_with(".lua") {
            EntryKind::Lua
        } else {
            EntryKind::Python
        }
    }

    /// True when [`AppnameVersion`-shaped] `(major, minor)` falls inside
    /// `[api.min, api.max]` (§25.2: compared against `appname_version()`;
    /// §25.6: out-of-range is REJECTED, not crashed).
    pub fn supports_abi(&self, major: u32, minor: u32) -> bool {
        // min bound: (major, minor) >= (min.major, min.minor)
        let min_minor = self.api_min.minor.unwrap_or(0);
        let above_min = (major, minor) >= (self.api_min.major, min_minor);
        // max bound: "N.x" caps the major only.
        let below_max = match self.api_max.minor {
            None => major <= self.api_max.major,
            Some(mm) => (major, minor) <= (self.api_max.major, mm),
        };
        above_min && below_max
    }

    /// Parse + strictly validate manifest bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        let v: Value =
            serde_json::from_slice(bytes).map_err(|e| ManifestError::NotJson(e.to_string()))?;
        let obj = v.as_object().ok_or(ManifestError::NotObject)?;

        // Unknown-key refusal (strictness-sweep discipline): the schema's
        // property set, closed.
        const KNOWN: &[&str] = &[
            "schema",
            "id",
            "version",
            "api",
            "entry",
            "permissions",
            "provides",
            "license",
            "author",
        ];
        for k in obj.keys() {
            if !KNOWN.contains(&k.as_str()) {
                return Err(ManifestError::UnknownKey(k.clone()));
            }
        }
        const REQUIRED: &[&str] = &[
            "schema",
            "id",
            "version",
            "api",
            "entry",
            "permissions",
            "provides",
        ];
        for k in REQUIRED {
            if !obj.contains_key(*k) {
                return Err(ManifestError::MissingField(k));
            }
        }

        match obj.get("schema").and_then(Value::as_str) {
            Some("appname.plugin.v1") => {}
            _ => return Err(ManifestError::WrongSchema),
        }

        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .ok_or(ManifestError::BadField("id"))?;
        if id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.')
        {
            return Err(ManifestError::BadId(id.to_string()));
        }

        let version = obj
            .get("version")
            .and_then(Value::as_str)
            .ok_or(ManifestError::BadField("version"))?;
        if version.is_empty() {
            return Err(ManifestError::BadField("version"));
        }

        let api = obj
            .get("api")
            .and_then(Value::as_object)
            .ok_or(ManifestError::BadField("api"))?;
        for k in api.keys() {
            if k != "min" && k != "max" {
                return Err(ManifestError::UnknownKey(format!("api.{k}")));
            }
        }
        let api_min = api
            .get("min")
            .and_then(Value::as_str)
            .and_then(|s| AbiBound::parse(s, false))
            .ok_or(ManifestError::BadField("api.min"))?;
        let api_max = api
            .get("max")
            .and_then(Value::as_str)
            .and_then(|s| AbiBound::parse(s, true))
            .ok_or(ManifestError::BadField("api.max"))?;

        let entry = obj
            .get("entry")
            .and_then(Value::as_str)
            .ok_or(ManifestError::BadField("entry"))?;
        // Entry hygiene: a bare filename with a sanctioned suffix. Path
        // separators, traversal, and empty stems are hostile-corpus
        // classes (W08), refused typed.
        let bad_entry = entry.contains('/')
            || entry.contains('\\')
            || entry.contains("..")
            || entry.starts_with('.');
        let good_suffix = ["wasm", "lua", "py"].iter().any(|sfx| {
            entry
                .strip_suffix(sfx)
                .and_then(|s| s.strip_suffix('.'))
                .is_some_and(|stem| !stem.is_empty())
        });
        if bad_entry || !good_suffix {
            return Err(ManifestError::BadEntry(entry.to_string()));
        }

        let perm_arr = obj
            .get("permissions")
            .and_then(Value::as_array)
            .ok_or(ManifestError::BadField("permissions"))?;
        let mut permissions = Vec::with_capacity(perm_arr.len());
        for p in perm_arr {
            let s = p.as_str().ok_or(ManifestError::BadField("permissions"))?;
            let cap = Capability::parse(s)
                .ok_or_else(|| ManifestError::UnknownPermission(s.to_string()))?;
            if permissions.contains(&cap) {
                return Err(ManifestError::DuplicatePermission(s.to_string()));
            }
            permissions.push(cap);
        }

        let prov_arr = obj
            .get("provides")
            .and_then(Value::as_array)
            .ok_or(ManifestError::BadField("provides"))?;
        let mut provides = Vec::with_capacity(prov_arr.len());
        for p in prov_arr {
            let po = p.as_object().ok_or(ManifestError::BadField("provides"))?;
            for k in po.keys() {
                if k != "type" && k != "id" {
                    return Err(ManifestError::UnknownKey(format!("provides[].{k}")));
                }
            }
            let kind = po
                .get("type")
                .and_then(Value::as_str)
                .and_then(ProvidedKind::parse)
                .ok_or(ManifestError::BadField("provides[].type"))?;
            let pid = po
                .get("id")
                .and_then(Value::as_str)
                .ok_or(ManifestError::BadField("provides[].id"))?;
            // §25.4/§10.2: provider ids are namespaced by the plugin —
            // they MUST extend the plugin id (`<plugin.id>.<name>`), which
            // is what makes the registry's `x.`-prefix rule enforceable
            // at registration time.
            if !pid
                .strip_prefix(id)
                .and_then(|rest| rest.strip_prefix('.'))
                .is_some_and(|name| !name.is_empty())
            {
                return Err(ManifestError::ProvidesOutsideNamespace(pid.to_string()));
            }
            provides.push(Provides {
                kind,
                id: pid.to_string(),
            });
        }
        // Every provider kind must be covered by a requested registration
        // capability (§25.3 row 1 — "listed at install" needs it listed).
        for p in &provides {
            let need = p.kind.required_capability();
            if !permissions.contains(&need) {
                return Err(ManifestError::ProvidesWithoutPermission {
                    id: p.id.clone(),
                    needs: need,
                });
            }
        }

        let license = match obj.get("license") {
            None => None,
            Some(v) => Some(
                v.as_str()
                    .ok_or(ManifestError::BadField("license"))?
                    .to_string(),
            ),
        };
        let author = match obj.get("author") {
            None => None,
            Some(v) => Some(
                v.as_str()
                    .ok_or(ManifestError::BadField("author"))?
                    .to_string(),
            ),
        };

        Ok(Self {
            id: id.to_string(),
            version: version.to_string(),
            api_min,
            api_max,
            entry: entry.to_string(),
            permissions,
            provides,
            license,
            author,
        })
    }
}

/// Typed manifest refusals (W08 hostile-manifest corpus classes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    NotJson(String),
    NotObject,
    WrongSchema,
    MissingField(&'static str),
    UnknownKey(String),
    BadField(&'static str),
    BadId(String),
    BadEntry(String),
    UnknownPermission(String),
    DuplicatePermission(String),
    ProvidesOutsideNamespace(String),
    ProvidesWithoutPermission { id: String, needs: Capability },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJson(e) => write!(f, "manifest is not JSON: {e}"),
            Self::NotObject => write!(f, "manifest is not a JSON object"),
            Self::WrongSchema => write!(f, "manifest schema is not appname.plugin.v1"),
            Self::MissingField(k) => write!(f, "manifest is missing required field {k}"),
            Self::UnknownKey(k) => write!(f, "manifest carries unknown key {k}"),
            Self::BadField(k) => write!(f, "manifest field {k} is malformed"),
            Self::BadId(id) => write!(f, "plugin id {id:?} violates ^[a-z0-9.]+$"),
            Self::BadEntry(e) => write!(
                f,
                "manifest entry {e:?} is not a bare plugin.wasm|plugin.lua|module.py filename"
            ),
            Self::UnknownPermission(p) => write!(f, "unknown permission {p:?}"),
            Self::DuplicatePermission(p) => write!(f, "duplicate permission {p:?}"),
            Self::ProvidesOutsideNamespace(id) => write!(
                f,
                "provides id {id:?} is not namespaced under the plugin id"
            ),
            Self::ProvidesWithoutPermission { id, needs } => write!(
                f,
                "provides id {id:?} needs permission {:?} which the manifest does not request",
                needs.as_str()
            ),
        }
    }
}

impl std::error::Error for ManifestError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_manifest() -> serde_json::Value {
        serde_json::json!({
            "schema": "appname.plugin.v1",
            "id": "org.example.blurpack",
            "version": "1.2.0",
            "api": { "min": "1.0", "max": "1.x" },
            "entry": "plugin.wasm",
            "permissions": ["filter.register"],
            "provides": [ { "type": "filter", "id": "org.example.blurpack.wobble" } ]
        })
    }

    fn parse(v: &serde_json::Value) -> Result<Manifest, ManifestError> {
        Manifest::parse(v.to_string().as_bytes())
    }

    #[test]
    fn the_spec_example_shape_parses() {
        let m = parse(&ok_manifest()).unwrap();
        assert_eq!(m.id, "org.example.blurpack");
        assert_eq!(m.entry_kind(), EntryKind::Wasm);
        assert_eq!(m.provides.len(), 1);
        assert!(m.supports_abi(1, 4));
        assert!(m.supports_abi(1, 99)); // "1.x" caps major only
        assert!(!m.supports_abi(2, 0));
        assert!(!m.supports_abi(0, 9));
    }

    #[test]
    fn unknown_key_refused_at_every_depth() {
        let mut v = ok_manifest();
        v["surprise"] = serde_json::json!(1);
        assert!(matches!(parse(&v), Err(ManifestError::UnknownKey(k)) if k == "surprise"));

        let mut v = ok_manifest();
        v["api"]["patch"] = serde_json::json!("0");
        assert!(matches!(parse(&v), Err(ManifestError::UnknownKey(k)) if k == "api.patch"));

        let mut v = ok_manifest();
        v["provides"][0]["extra"] = serde_json::json!(true);
        assert!(matches!(parse(&v), Err(ManifestError::UnknownKey(k)) if k == "provides[].extra"));
    }

    #[test]
    fn hostile_entries_refused() {
        for evil in [
            "../../../evil.wasm",
            "/etc/plugin.wasm",
            "a\\b.lua",
            ".wasm",
            "plugin.exe",
            "plugin.wasm.txt",
            "",
        ] {
            let mut v = ok_manifest();
            v["entry"] = serde_json::json!(evil);
            assert!(
                matches!(parse(&v), Err(ManifestError::BadEntry(_))),
                "entry {evil:?} must refuse"
            );
        }
    }

    #[test]
    fn id_pattern_enforced() {
        for evil in [
            "Org.Example",
            "org example",
            "org/example",
            "",
            "órg.example",
        ] {
            let mut v = ok_manifest();
            v["id"] = serde_json::json!(evil);
            // provides namespace check depends on id; BadId fires first.
            assert!(
                matches!(parse(&v), Err(ManifestError::BadId(_))),
                "id {evil:?} must refuse"
            );
        }
    }

    #[test]
    fn unknown_permission_refused() {
        let mut v = ok_manifest();
        v["permissions"] = serde_json::json!(["filter.register", "root.everything"]);
        assert!(matches!(
            parse(&v),
            Err(ManifestError::UnknownPermission(p)) if p == "root.everything"
        ));
    }

    #[test]
    fn provides_must_be_namespaced_and_permitted() {
        let mut v = ok_manifest();
        v["provides"][0]["id"] = serde_json::json!("com.other.thing");
        assert!(matches!(
            parse(&v),
            Err(ManifestError::ProvidesOutsideNamespace(_))
        ));

        // Exact-id (no suffix) is also outside: provider ids EXTEND the id.
        let mut v = ok_manifest();
        v["provides"][0]["id"] = serde_json::json!("org.example.blurpack");
        assert!(matches!(
            parse(&v),
            Err(ManifestError::ProvidesOutsideNamespace(_))
        ));

        let mut v = ok_manifest();
        v["provides"][0] = serde_json::json!({ "type": "tool", "id": "org.example.blurpack.t" });
        assert!(matches!(
            parse(&v),
            Err(ManifestError::ProvidesWithoutPermission {
                needs: Capability::ToolRegister,
                ..
            })
        ));
    }

    #[test]
    fn api_bounds_parse_strictly() {
        for evil in ["1", "1.", ".1", "1.x.2", "x.1", "1.-1", "01a.0", "1.0xg"] {
            let mut v = ok_manifest();
            v["api"]["min"] = serde_json::json!(evil);
            assert!(parse(&v).is_err(), "api.min {evil:?} must refuse");
        }
        // "x" minor is legal on max only.
        let mut v = ok_manifest();
        v["api"]["min"] = serde_json::json!("1.x");
        assert!(matches!(parse(&v), Err(ManifestError::BadField("api.min"))));
    }

    #[test]
    fn schema_const_and_required_fields() {
        let mut v = ok_manifest();
        v["schema"] = serde_json::json!("appname.plugin.v2");
        assert!(matches!(parse(&v), Err(ManifestError::WrongSchema)));

        for k in [
            "schema",
            "id",
            "version",
            "api",
            "entry",
            "permissions",
            "provides",
        ] {
            let mut v = ok_manifest();
            v.as_object_mut().unwrap().remove(k);
            assert!(
                matches!(parse(&v), Err(ManifestError::MissingField(m)) if m == k),
                "missing {k} must refuse"
            );
        }
    }
}
