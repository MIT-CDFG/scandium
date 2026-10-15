// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Plugin discovery and the load-order pipeline (ES §25.6 [NORMATIVE]).
//!
//! Discovery: scan the configured directories in order (system dirs
//! then the user dir; extra dirs from `[plugins] dirs`); each plugin is
//! an unpacked directory named `<id>-<version>`. On id collision the
//! USER dir shadows the system dir; among versions in ONE dir the
//! highest version wins and the others are ignored with a log line.
//!
//! Load order per plugin: manifest parse → dir-name/id agreement → ABI
//! range check → signature classification → permission diff against
//! stored grants (new permissions re-prompt) → host init LAZY
//! (registration metadata now, sandbox instantiation on first
//! invocation). When a tier's cargo feature is disabled the tier is
//! DROPPED (§48.4) and [`crate::plugin::host::host_availability`]
//! reports [`crate::plugin::host::HostAvailability::Dropped`]. Load
//! failures NEVER abort the scan; the plugin lists as
//! [`PluginStatus::Failed`] with its typed error.
//!
//! Signature posture v1 (honest, lane-log recorded): `signature.json`
//! is parsed and shape-checked (`appname.sig.v1`, 32/64-byte base64
//! fields), but ed25519 VERIFICATION is record-first-held with the rest
//! of the crypto surface (no ed25519 dep record). Until it lands, a
//! signature-carrying package classifies as
//! [`SignatureState::PresentUnverified`] and follows the UNSIGNED
//! consent path (§25.5: explicit warning + deliberate confirm) — a
//! signature that cannot be checked must not buy trust. Key PINNING
//! still works bytes-for-bytes: a changed `publicKey` vs the stored pin
//! re-prompts (§25.5/§25.6), verification or not.

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::plugin::manifest::{Manifest, ManifestError};
use crate::plugin::permissions::Capability;
use crate::plugin::state::{PluginState, StateFile};

/// The §25.6 scan set, in shadowing order (LOWEST precedence first: a
/// later dir's plugin shadows an earlier dir's on id collision — the
/// user dir comes last).
#[derive(Debug, Clone, Default)]
pub struct ScanDirs {
    pub dirs: Vec<PathBuf>,
}

impl ScanDirs {
    /// The platform defaults (§25.6 table) + extra `[plugins] dirs`.
    /// Kept data-driven (no cfg!) so tests cover every row on one OS;
    /// the embedder passes the platform row.
    pub fn new(system: Vec<PathBuf>, user: PathBuf, extra: Vec<PathBuf>) -> Self {
        let mut dirs = system;
        dirs.push(user);
        dirs.extend(extra);
        Self { dirs }
    }
}

/// Signature classification (verification record-first-held, module
/// docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureState {
    /// No `signature.json` — §25.5 "unsigned": warn-and-confirm path.
    Absent,
    /// Present, shape-valid; ed25519 check pending its record. Treated
    /// as unsigned for consent purposes (module docs). Carries the
    /// base64 `publicKey` for pinning.
    PresentUnverified { public_key: String },
    /// Present but shape-invalid — install refused (§25.5 posture: a
    /// malformed signature is a verification failure, not "unsigned").
    Malformed(String),
}

/// Why a plugin failed the pipeline (typed; the Plugin Manager surfaces
/// these verbatim through their MessageIds later — O-UI's chrome).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadRefusal {
    ManifestMissing,
    Manifest(ManifestError),
    /// Directory name does not equal `<id>-<version>` from the manifest
    /// (an id-spoofing corpus class: the dir name is what shadowing
    /// keys on, so disagreement is hostile).
    DirNameMismatch {
        dir: String,
        expected: String,
    },
    /// ABI out of range: "Requires Appname ABI ≥/≤ x.y" (§25.6 —
    /// rejected, never crashed, listed grayed).
    AbiOutOfRange {
        min: String,
        max: String,
    },
    /// Malformed signature.json (§25.5: refused).
    Signature(String),
    /// The stored key pin differs from the package's key (§25.5 key
    /// change — re-prompt, not silent accept).
    KeyChanged,
    /// Entry file named by the manifest is absent from the package.
    EntryMissing(String),
}

impl fmt::Display for LoadRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ManifestMissing => write!(f, "manifest.json missing"),
            Self::Manifest(e) => write!(f, "manifest refused: {e}"),
            Self::DirNameMismatch { dir, expected } => {
                write!(f, "directory {dir:?} does not match manifest {expected:?}")
            }
            Self::AbiOutOfRange { min, max } => {
                write!(f, "plugin targets ABI [{min}, {max}], outside this build")
            }
            Self::Signature(e) => write!(f, "signature.json refused: {e}"),
            Self::KeyChanged => write!(f, "publicKey differs from the pinned key"),
            Self::EntryMissing(e) => write!(f, "entry {e:?} missing from the package"),
        }
    }
}

/// Where one scanned plugin landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginStatus {
    /// Loaded metadata; enabled per state; sandbox instantiates lazily.
    Enabled,
    /// Known and intact, not enabled (fresh installs start here until
    /// the user's deliberate enable — which is also where the §25.5
    /// unsigned confirm and the permission prompt happen).
    Disabled,
    /// Enabled in state but NEW permissions appeared (§25.6 re-prompt):
    /// inert until the user rules on the listed capabilities.
    NeedsGrant(Vec<Capability>),
    /// Failed the pipeline (typed).
    Failed(LoadRefusal),
    /// Shadowed by a same-id plugin in a higher-precedence dir, or a
    /// lower version in the same dir; carries the winner's dir.
    Shadowed { by: PathBuf },
}

/// One scanned plugin.
#[derive(Debug, Clone)]
pub struct PluginRecord {
    /// Directory the plugin lives in.
    pub dir: PathBuf,
    /// Manifest, when it parsed (Failed(ManifestMissing/Manifest) has none).
    pub manifest: Option<Manifest>,
    pub signature: Option<SignatureState>,
    pub status: PluginStatus,
    /// Scan log lines for this record (shadow decisions, dropped grant
    /// dirs, …) — the §25.6 "ignored with a log line" surface.
    pub log: Vec<String>,
}

impl PluginRecord {
    pub fn id(&self) -> Option<&str> {
        self.manifest.as_ref().map(|m| m.id.as_str())
    }
}

/// The scan result: every record + the state file (updated with silent
/// grants for newly seen enabled plugins; persisting is the caller's
/// explicit step).
#[derive(Debug)]
pub struct PluginRegistry {
    pub records: Vec<PluginRecord>,
}

impl PluginRegistry {
    pub fn get(&self, id: &str) -> Option<&PluginRecord> {
        self.records
            .iter()
            .find(|r| r.id() == Some(id) && !matches!(r.status, PluginStatus::Shadowed { .. }))
    }
}

/// Compare two `<id>-<version>` version suffixes: numeric dot-tuples
/// compare numerically, longer tuple wins ties, non-numeric parts fall
/// back to byte order (deliberately simple; the repo index uses real
/// semver, a fold-in note).
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let nums =
        |s: &str| -> Vec<Option<u64>> { s.split('.').map(|p| p.parse::<u64>().ok()).collect() };
    let (va, vb) = (nums(a), nums(b));
    for i in 0..va.len().max(vb.len()) {
        let x = va.get(i).cloned().flatten();
        let y = vb.get(i).cloned().flatten();
        match (x, y) {
            (Some(x), Some(y)) => match x.cmp(&y) {
                std::cmp::Ordering::Equal => {}
                other => return other,
            },
            (None, None) => {
                // Non-numeric or absent on both sides: byte order of the
                // raw parts (absent < present).
                let xa = a.split('.').nth(i).unwrap_or("");
                let xb = b.split('.').nth(i).unwrap_or("");
                match xa.cmp(xb) {
                    std::cmp::Ordering::Equal => {}
                    other => return other,
                }
            }
            // Numeric beats non-numeric ("2" > "beta").
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (None, Some(_)) => return std::cmp::Ordering::Less,
        }
    }
    std::cmp::Ordering::Equal
}

/// Parse + shape-check `signature.json` (verification held, module docs).
fn classify_signature(dir: &Path) -> SignatureState {
    let path = dir.join("signature.json");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return SignatureState::Absent,
    };
    let v: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => return SignatureState::Malformed(format!("not JSON: {e}")),
    };
    if v.get("schema").and_then(Value::as_str) != Some("appname.sig.v1") {
        return SignatureState::Malformed("schema is not appname.sig.v1".into());
    }
    let b64_len = |field: &str| -> Result<(String, usize), String> {
        let s = v
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{field} missing"))?;
        // Decoded length from base64 length (strict, padded form).
        let trimmed = s.trim_end_matches('=');
        let bits = trimmed.len() * 6;
        Ok((s.to_string(), bits / 8))
    };
    match (b64_len("publicKey"), b64_len("sig")) {
        (Err(e), _) | (_, Err(e)) => SignatureState::Malformed(e),
        (Ok((pk, 32)), Ok((_, 64))) => SignatureState::PresentUnverified { public_key: pk },
        (Ok((_, n)), Ok(_)) if n != 32 => {
            SignatureState::Malformed(format!("publicKey decodes to {n} bytes, not 32"))
        }
        (Ok(_), Ok((_, n))) => {
            SignatureState::Malformed(format!("sig decodes to {n} bytes, not 64"))
        }
    }
}

/// Run the pipeline over one plugin directory (no shadowing decision —
/// the scan does that across dirs).
fn load_one(dir: &Path, abi: (u32, u32), state: &StateFile) -> PluginRecord {
    let mut rec = PluginRecord {
        dir: dir.to_path_buf(),
        manifest: None,
        signature: None,
        status: PluginStatus::Failed(LoadRefusal::ManifestMissing),
        log: Vec::new(),
    };
    let manifest_bytes = match std::fs::read(dir.join("manifest.json")) {
        Ok(b) => b,
        Err(_) => return rec,
    };
    let manifest = match Manifest::parse(&manifest_bytes) {
        Ok(m) => m,
        Err(e) => {
            rec.status = PluginStatus::Failed(LoadRefusal::Manifest(e));
            return rec;
        }
    };
    // Dir-name/id agreement (module docs: shadowing keys on the name).
    let expected = format!("{}-{}", manifest.id, manifest.version);
    let dir_name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if dir_name != expected {
        rec.status = PluginStatus::Failed(LoadRefusal::DirNameMismatch {
            dir: dir_name,
            expected,
        });
        rec.manifest = Some(manifest);
        return rec;
    }
    // ABI range (§25.6: rejected, never crashed).
    if !manifest.supports_abi(abi.0, abi.1) {
        rec.status = PluginStatus::Failed(LoadRefusal::AbiOutOfRange {
            min: format!(
                "{}.{}",
                manifest.api_min.major,
                manifest.api_min.minor.unwrap_or(0)
            ),
            max: match manifest.api_max.minor {
                Some(m) => format!("{}.{}", manifest.api_max.major, m),
                None => format!("{}.x", manifest.api_max.major),
            },
        });
        rec.manifest = Some(manifest);
        return rec;
    }
    // Entry presence.
    if !dir.join(&manifest.entry).is_file() {
        rec.status = PluginStatus::Failed(LoadRefusal::EntryMissing(manifest.entry.clone()));
        rec.manifest = Some(manifest);
        return rec;
    }
    // Signature classification + key pin.
    let sig = classify_signature(dir);
    if let SignatureState::Malformed(e) = &sig {
        rec.status = PluginStatus::Failed(LoadRefusal::Signature(e.clone()));
        rec.signature = Some(sig.clone());
        rec.manifest = Some(manifest);
        return rec;
    }
    if let (SignatureState::PresentUnverified { public_key }, Some(st)) =
        (&sig, state.get(&manifest.id))
    {
        if !st.public_key.is_empty() && &st.public_key != public_key {
            rec.status = PluginStatus::Failed(LoadRefusal::KeyChanged);
            rec.signature = Some(sig.clone());
            rec.manifest = Some(manifest);
            return rec;
        }
    }
    // Permission diff against stored grants.
    let status = match state.get(&manifest.id) {
        Some(st) if st.enabled => {
            let missing = st.granted.ungranted_of(&manifest.permissions);
            if missing.is_empty() {
                PluginStatus::Enabled
            } else {
                PluginStatus::NeedsGrant(missing)
            }
        }
        _ => PluginStatus::Disabled,
    };
    rec.status = status;
    rec.signature = Some(sig);
    rec.manifest = Some(manifest);
    rec
}

/// Scan every configured directory (ES §25.6 discovery). `abi` is the
/// host's `(abi_major, abi_minor)` — the C ABI door passes its own
/// version, keeping this crate free of a cabi dependency (no cycle).
pub fn scan(dirs: &ScanDirs, abi: (u32, u32), state: &StateFile) -> PluginRegistry {
    let mut records: Vec<PluginRecord> = Vec::new();
    for dir in &dirs.dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue; // absent scan dirs are normal
        };
        let mut names: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        names.sort(); // deterministic scan order
        for pdir in names {
            let rec = load_one(&pdir, abi, state);
            records.push(rec);
        }
    }
    // Shadowing: same id — later dir wins (user shadows system);
    // within one dir — highest version wins. Walk winners per id.
    let ids: Vec<String> = records
        .iter()
        .filter_map(|r| r.id().map(str::to_string))
        .collect();
    let mut seen: Vec<&str> = Vec::new();
    for id in ids.iter().map(String::as_str) {
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        let contenders: Vec<usize> = records
            .iter()
            .enumerate()
            .filter(|(_, r)| r.id() == Some(id))
            .map(|(i, _)| i)
            .collect();
        if contenders.len() < 2 {
            continue;
        }
        // Winner: last scan dir containing the id (highest precedence),
        // then highest version within that dir.
        let dir_of = |i: usize| records[i].dir.parent().map(Path::to_path_buf);
        let last_dir = contenders.iter().rev().find_map(|&i| dir_of(i));
        let winner = contenders
            .iter()
            .copied()
            .filter(|&i| dir_of(i) == last_dir)
            .max_by(|&a, &b| {
                let ver = |i: usize| {
                    records[i]
                        .manifest
                        .as_ref()
                        .map(|m| m.version.clone())
                        .unwrap_or_default()
                };
                version_cmp(&ver(a), &ver(b))
            })
            .expect("contenders is non-empty");
        let winner_dir = records[winner].dir.clone();
        for &i in &contenders {
            if i != winner {
                records[i].status = PluginStatus::Shadowed {
                    by: winner_dir.clone(),
                };
                records[i]
                    .log
                    .push(format!("shadowed by {}", winner_dir.display()));
            }
        }
    }
    PluginRegistry { records }
}

/// Enable a plugin: the deliberate user step where silent registration
/// grants are recorded (§25.3 row 1) and the key pin is stored. Returns
/// the capabilities that still need explicit grants (fs/net/clipboard —
/// the caller prompts). State persistence is the caller's save.
pub fn enable(
    registry: &PluginRegistry,
    state: &mut StateFile,
    id: &str,
) -> Result<Vec<Capability>, String> {
    let rec = registry
        .get(id)
        .ok_or_else(|| format!("no scanned plugin with id {id}"))?;
    let manifest = match (&rec.status, &rec.manifest) {
        (PluginStatus::Failed(e), _) => return Err(format!("plugin {id} failed: {e}")),
        (_, Some(m)) => m,
        (_, None) => return Err(format!("plugin {id} has no manifest")),
    };
    let public_key = match &rec.signature {
        Some(SignatureState::PresentUnverified { public_key }) => public_key.clone(),
        _ => String::new(),
    };
    if state.get(id).is_none() {
        state.plugins.push(PluginState {
            id: id.to_string(),
            version: manifest.version.clone(),
            enabled: false,
            granted: Default::default(),
            public_key: String::new(),
        });
    }
    let st = state.get_mut(id).expect("just ensured");
    st.enabled = true;
    st.version = manifest.version.clone();
    st.public_key = public_key;
    st.granted.grant_silent(&manifest.permissions);
    Ok(st.granted.ungranted_of(&manifest.permissions))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABI: (u32, u32) = (1, 5);

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "appname-lc-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_plugin(root: &Path, id: &str, version: &str, entry: &str, perms: &str) -> PathBuf {
        let dir = root.join(format!("{id}-{version}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            format!(
                r#"{{"schema":"appname.plugin.v1","id":"{id}","version":"{version}",
                     "api":{{"min":"1.0","max":"1.x"}},"entry":"{entry}",
                     "permissions":{perms},
                     "provides":[{{"type":"filter","id":"{id}.f"}}]}}"#
            ),
        )
        .unwrap();
        std::fs::write(dir.join(entry), b"\0asm").unwrap();
        dir
    }

    #[test]
    fn scan_load_enable_flow() {
        let sys = tmp("sys");
        write_plugin(
            &sys,
            "org.example.a",
            "1.0.0",
            "plugin.wasm",
            r#"["filter.register","fs.read"]"#,
        );
        let dirs = ScanDirs::new(vec![sys], tmp("user"), vec![]);
        let mut state = StateFile::default();
        let reg = scan(&dirs, ABI, &state);
        assert_eq!(reg.records.len(), 1);
        assert_eq!(
            reg.get("org.example.a").unwrap().status,
            PluginStatus::Disabled
        );
        // Enable: silent grant recorded, fs.read left to prompt.
        let to_prompt = enable(&reg, &mut state, "org.example.a").unwrap();
        assert_eq!(to_prompt, vec![Capability::FsRead]);
        assert!(state.get("org.example.a").unwrap().enabled);
        // Re-scan now shows NeedsGrant (enabled but fs.read unruled).
        let reg2 = scan(&dirs, ABI, &state);
        assert_eq!(
            reg2.get("org.example.a").unwrap().status,
            PluginStatus::NeedsGrant(vec![Capability::FsRead])
        );
    }

    #[test]
    fn user_dir_shadows_system_dir() {
        let sys = tmp("sys2");
        let user = tmp("user2");
        let sys_dir = write_plugin(
            &sys,
            "org.example.a",
            "1.0.0",
            "plugin.wasm",
            r#"["filter.register"]"#,
        );
        let user_dir = write_plugin(
            &user,
            "org.example.a",
            "0.9.0",
            "plugin.lua",
            r#"["filter.register"]"#,
        );
        let dirs = ScanDirs::new(vec![sys], user, vec![]);
        let reg = scan(&dirs, ABI, &StateFile::default());
        let by_dir = |d: &PathBuf| {
            reg.records
                .iter()
                .find(|r| &r.dir == d)
                .unwrap()
                .status
                .clone()
        };
        // User wins even at a LOWER version (§25.6: shadowing is by dir
        // precedence first, version second within one dir).
        assert_eq!(by_dir(&user_dir), PluginStatus::Disabled);
        assert!(matches!(by_dir(&sys_dir), PluginStatus::Shadowed { .. }));
    }

    #[test]
    fn highest_version_wins_within_one_dir() {
        let sys = tmp("sys3");
        let older = write_plugin(
            &sys,
            "org.example.a",
            "1.2.0",
            "plugin.wasm",
            r#"["filter.register"]"#,
        );
        let newer = write_plugin(
            &sys,
            "org.example.a",
            "1.10.0",
            "plugin.wasm",
            r#"["filter.register"]"#,
        );
        let dirs = ScanDirs::new(vec![sys], tmp("user3"), vec![]);
        let reg = scan(&dirs, ABI, &StateFile::default());
        let status_of = |d: &PathBuf| reg.records.iter().find(|r| &r.dir == d).unwrap();
        // 1.10.0 > 1.2.0 numerically (not lexically).
        assert_eq!(status_of(&newer).status, PluginStatus::Disabled);
        assert!(matches!(
            status_of(&older).status,
            PluginStatus::Shadowed { .. }
        ));
        assert!(!status_of(&older).log.is_empty());
    }

    #[test]
    fn failures_never_abort_the_scan() {
        let sys = tmp("sys4");
        // Hostile: manifest missing.
        std::fs::create_dir_all(sys.join("junk-0")).unwrap();
        // Hostile: dir-name/id mismatch.
        let evil = sys.join("org.example.spoof-1.0.0");
        std::fs::create_dir_all(&evil).unwrap();
        std::fs::write(
            evil.join("manifest.json"),
            br#"{"schema":"appname.plugin.v1","id":"org.example.real","version":"1.0.0",
                "api":{"min":"1.0","max":"1.x"},"entry":"plugin.wasm",
                "permissions":["filter.register"],
                "provides":[{"type":"filter","id":"org.example.real.f"}]}"#,
        )
        .unwrap();
        // Fine plugin alongside.
        write_plugin(
            &sys,
            "org.example.ok",
            "1.0.0",
            "plugin.lua",
            r#"["filter.register"]"#,
        );
        let dirs = ScanDirs::new(vec![sys], tmp("user4"), vec![]);
        let reg = scan(&dirs, ABI, &StateFile::default());
        assert_eq!(reg.records.len(), 3);
        assert!(reg
            .records
            .iter()
            .any(|r| matches!(r.status, PluginStatus::Failed(LoadRefusal::ManifestMissing))));
        assert!(reg.records.iter().any(|r| matches!(
            r.status,
            PluginStatus::Failed(LoadRefusal::DirNameMismatch { .. })
        )));
        assert_eq!(
            reg.get("org.example.ok").unwrap().status,
            PluginStatus::Disabled
        );
    }

    #[test]
    fn abi_range_rejected_not_crashed() {
        let sys = tmp("sys5");
        let dir = sys.join("org.example.future-1.0.0");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            br#"{"schema":"appname.plugin.v1","id":"org.example.future","version":"1.0.0",
                "api":{"min":"2.0","max":"2.x"},"entry":"plugin.wasm",
                "permissions":["filter.register"],
                "provides":[{"type":"filter","id":"org.example.future.f"}]}"#,
        )
        .unwrap();
        std::fs::write(dir.join("plugin.wasm"), b"\0asm").unwrap();
        let dirs = ScanDirs::new(vec![sys], tmp("user5"), vec![]);
        let reg = scan(&dirs, ABI, &StateFile::default());
        assert!(matches!(
            &reg.records[0].status,
            PluginStatus::Failed(LoadRefusal::AbiOutOfRange { min, .. }) if min == "2.0"
        ));
    }

    #[test]
    fn key_pin_change_refuses() {
        let sys = tmp("sys6");
        let dir = write_plugin(
            &sys,
            "org.example.a",
            "1.0.0",
            "plugin.wasm",
            r#"["filter.register"]"#,
        );
        // 32-byte key + 64-byte sig, base64 (constructed, not hand-typed:
        // 32 = 10 triples + 2 bytes, 64 = 21 triples + 1 byte of 'A').
        let sig = serde_json::json!({
            "schema": "appname.sig.v1",
            "publicKey": format!("{}QUE=", "QUFB".repeat(10)),
            "sig": format!("{}QQ==", "QUFB".repeat(21))
        });
        std::fs::write(dir.join("signature.json"), sig.to_string()).unwrap();
        let mut state = StateFile::default();
        state.plugins.push(PluginState {
            id: "org.example.a".into(),
            version: "0.9.0".into(),
            enabled: true,
            granted: Default::default(),
            public_key: "DIFFERENTKEYDIFFERENTKEYDIFFERENTKEYDIFFo=".into(),
        });
        let dirs = ScanDirs::new(vec![sys], tmp("user6"), vec![]);
        let reg = scan(&dirs, ABI, &state);
        assert!(matches!(
            reg.records[0].status,
            PluginStatus::Failed(LoadRefusal::KeyChanged)
        ));
    }

    #[test]
    fn malformed_signature_refuses() {
        let sys = tmp("sys7");
        let dir = write_plugin(
            &sys,
            "org.example.a",
            "1.0.0",
            "plugin.wasm",
            r#"["filter.register"]"#,
        );
        std::fs::write(
            dir.join("signature.json"),
            br#"{"schema":"appname.sig.v1","publicKey":"c2hvcnQ=","sig":"c2hvcnQ="}"#,
        )
        .unwrap();
        let dirs = ScanDirs::new(vec![sys], tmp("user7"), vec![]);
        let reg = scan(&dirs, ABI, &StateFile::default());
        assert!(matches!(
            reg.records[0].status,
            PluginStatus::Failed(LoadRefusal::Signature(_))
        ));
    }
}
