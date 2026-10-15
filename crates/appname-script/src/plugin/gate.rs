// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The capability gate — every sandboxed host petition passes here
//! before anything happens (ES §25.3 [NORMATIVE]; §10.2 stage-5).
//!
//! Policy pins for the v1 sandboxed tier (plan §2.5 row):
//!
//! * **fs** — the plugin's SESSION STORE (relative petitions) plus
//!   explicitly granted directories (absolute petitions), read and
//!   write scopes independent. Everything else:
//!   `error.permission.filesystem_denied` → `APPNAME_ERR_PERMISSION`.
//! * **net** — **DENY**, unconditionally. [`NetPolicy::Deny`] is the v1
//!   constant: ES §30.1 `[privacy] network_features = false` is the
//!   default and [NORMATIVE §25.3] "hard-gates `net` for every tier
//!   regardless of grants"; the grant plumbing exists and persists
//!   (§25.6) so flipping the preference later is a policy change, not a
//!   model change — but this tier v1 refuses even a granted `net`.
//!   Wire: `error.permission.network_denied`.
//! * **clipboard** — no sandboxed host call in v1 (§25.3 table:
//!   sandboxed clipboard = off); a petition refuses typed.
//! * **registration** — a provider registration must be covered by the
//!   silently-granted registration capability AND name a manifest
//!   `provides` id (namespaced under the plugin id — which satisfies
//!   the §10.2 registry rule that plugin kinds are prefixed, since the
//!   host registers them as `x.<provider id>`).
//! * **apply** — `appname.command.v1` envelopes pass through to the one
//!   door unmodified; catalog kinds carry `requires: []` (any caller,
//!   ES §10.2) and the engine's stage-5 check stays authoritative. The
//!   gate refuses only envelopes that are not JSON objects (the host
//!   never forwards garbage bytes to the door).
//!
//! Denials are TYPED, never panics, never silent: the W08 corpus
//! (`tests/plugin_safety.rs`) drives every class listed here.

use std::fmt;
use std::path::PathBuf;

use crate::plugin::manifest::Manifest;
use crate::plugin::permissions::{Capability, GrantSet};
use crate::plugin::session::{BrokeredDir, PathRefusal};

/// Network policy. v1 pins [`NetPolicy::Deny`] (module docs); the enum
/// exists so the §30.1 preference can surface later without reshaping
/// the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetPolicy {
    /// The v1 constant: refuse every network petition.
    #[default]
    Deny,
}

/// A typed permission denial. Maps to `APPNAME_ERR_PERMISSION` at the
/// ABI boundary; [`PermissionError::wire_id`] gives the registered
/// MessageId where one exists (contracts/message_ids.yaml).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionError {
    /// Filesystem access refused: no covering grant, or the petition
    /// escaped its confinement (the refusal carries which).
    FsDenied {
        plugin: String,
        petition: String,
        write: bool,
        refusal: Option<PathRefusal>,
    },
    /// Network refused (v1: always).
    NetDenied { plugin: String },
    /// Clipboard refused (no sandboxed clipboard call in v1).
    ClipboardDenied { plugin: String },
    /// Provider registration refused: capability not granted, or the id
    /// is not a manifest `provides` entry.
    RegistrationDenied {
        plugin: String,
        provider_id: String,
        reason: RegistrationRefusal,
    },
    /// A mutation petition that was not a JSON object envelope.
    MalformedEnvelope { plugin: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationRefusal {
    CapabilityNotGranted(Capability),
    NotInProvides,
}

impl PermissionError {
    /// The registered wire MessageId for surfaces that show the denial
    /// (both ids are shipped-v1 in `contracts/message_ids.yaml`; the
    /// catalog entries are an O-I18N companion, flagged in the lane
    /// log). Registration/envelope refusals surface through the
    /// engine's ordinary error JSON instead and carry no id here.
    pub fn wire_id(&self) -> Option<&'static str> {
        match self {
            Self::FsDenied { .. } => Some("error.permission.filesystem_denied"),
            Self::NetDenied { .. } => Some("error.permission.network_denied"),
            // Clipboard has NO registered MessageId (Appendix H mints
            // none) — do NOT borrow the network id (a client would read
            // "network denied" for a clipboard ask). A dedicated id needs
            // the mint-and-log path, out of scope for this slice; until
            // then the denial surfaces through the ordinary error path
            // (Display), like RegistrationDenied/MalformedEnvelope.
            Self::ClipboardDenied { .. }
            | Self::RegistrationDenied { .. }
            | Self::MalformedEnvelope { .. } => None,
        }
    }
}

impl fmt::Display for PermissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FsDenied {
                plugin,
                petition,
                write,
                refusal,
            } => {
                let mode = if *write { "write" } else { "read" };
                match refusal {
                    Some(r) => write!(
                        f,
                        "plugin {plugin}: filesystem {mode} of {petition:?} denied ({r})"
                    ),
                    None => write!(
                        f,
                        "plugin {plugin}: filesystem {mode} of {petition:?} denied (no covering grant)"
                    ),
                }
            }
            Self::NetDenied { plugin } => {
                write!(
                    f,
                    "plugin {plugin}: network access denied (v1 sandboxed tier: net = deny)"
                )
            }
            Self::ClipboardDenied { plugin } => {
                write!(
                    f,
                    "plugin {plugin}: clipboard access denied (no sandboxed clipboard in v1)"
                )
            }
            Self::RegistrationDenied {
                plugin,
                provider_id,
                reason,
            } => match reason {
                RegistrationRefusal::CapabilityNotGranted(c) => write!(
                    f,
                    "plugin {plugin}: registering {provider_id:?} denied (capability {} not granted)",
                    c.as_str()
                ),
                RegistrationRefusal::NotInProvides => write!(
                    f,
                    "plugin {plugin}: registering {provider_id:?} denied (not a manifest provides entry)"
                ),
            },
            Self::MalformedEnvelope { plugin } => {
                write!(
                    f,
                    "plugin {plugin}: mutation petition was not a JSON object envelope"
                )
            }
        }
    }
}

impl std::error::Error for PermissionError {}

/// The per-plugin gate: session store + granted dirs + policies,
/// constructed by the lifecycle from the manifest and the grant store.
#[derive(Debug)]
pub struct Gatekeeper {
    plugin_id: String,
    session: BrokeredDir,
    read_grants: Vec<BrokeredDir>,
    write_grants: Vec<BrokeredDir>,
    net: NetPolicy,
    manifest_provides: Vec<String>,
    granted: GrantSet,
}

impl Gatekeeper {
    /// Build the gate. Granted directories that cannot be established
    /// (missing, unreadable) are DROPPED from the live set — a stale
    /// grant narrows to nothing rather than failing the load (§25.6
    /// "load failures never abort startup" posture), and the drop list
    /// is returned for the Plugin Manager to surface.
    pub fn new(
        manifest: &Manifest,
        granted: GrantSet,
        session: BrokeredDir,
    ) -> (Self, Vec<PathBuf>) {
        let mut dropped = Vec::new();
        let mut establish_all = |dirs: Vec<&PathBuf>| -> Vec<BrokeredDir> {
            dirs.into_iter()
                .filter_map(|p| match BrokeredDir::establish(p) {
                    Ok(d) => Some(d),
                    Err(_) => {
                        dropped.push(p.clone());
                        None
                    }
                })
                .collect()
        };
        let read_grants = establish_all(granted.fs_read_dirs().collect());
        let write_grants = establish_all(granted.fs_write_dirs().collect());
        (
            Self {
                plugin_id: manifest.id.clone(),
                session,
                read_grants,
                write_grants,
                net: NetPolicy::default(),
                manifest_provides: manifest.provides.iter().map(|p| p.id.clone()).collect(),
                granted,
            },
            dropped,
        )
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    /// The session-store root (host bindings preopen exactly this).
    pub fn session_root(&self) -> &std::path::Path {
        self.session.root()
    }

    fn fs_denied(
        &self,
        petition: &str,
        write: bool,
        refusal: Option<PathRefusal>,
    ) -> PermissionError {
        PermissionError::FsDenied {
            plugin: self.plugin_id.clone(),
            petition: petition.to_string(),
            write,
            refusal,
        }
    }

    /// Petition a filesystem path. RELATIVE petitions address the
    /// session store (always available — the "session store" half of
    /// the row pin). ABSOLUTE petitions must fall inside a granted
    /// directory of the right scope AND the matching `fs.read`/`fs.write`
    /// capability must be granted. Returns the real path to use.
    pub fn petition_fs(&self, petition: &str, write: bool) -> Result<PathBuf, PermissionError> {
        let is_abs = std::path::Path::new(petition).is_absolute();
        if !is_abs {
            return self
                .session
                .resolve_relative(petition)
                .map_err(|r| self.fs_denied(petition, write, Some(r)));
        }
        let cap = if write {
            Capability::FsWrite
        } else {
            Capability::FsRead
        };
        if !self.granted.has_capability(cap) {
            return Err(self.fs_denied(petition, write, None));
        }
        let grants = if write {
            &self.write_grants
        } else {
            &self.read_grants
        };
        let mut last_refusal = None;
        for dir in grants {
            match dir.admit_absolute(petition) {
                Ok(p) => return Ok(p),
                Err(r) => last_refusal = Some(r),
            }
        }
        // Escape-class refusals (traversal, symlink) are more useful to
        // log than "no grant" when a grant candidate existed.
        let refusal = last_refusal
            .filter(|r| matches!(r, PathRefusal::SymlinkEscape(_)) || grants.len() == 1);
        Err(self.fs_denied(petition, write, refusal))
    }

    /// Petition network access. v1: deny, unconditionally (module docs).
    pub fn petition_net(&self) -> Result<(), PermissionError> {
        match self.net {
            NetPolicy::Deny => Err(PermissionError::NetDenied {
                plugin: self.plugin_id.clone(),
            }),
        }
    }

    /// Petition clipboard access. v1 sandboxed tier: deny.
    pub fn petition_clipboard(&self) -> Result<(), PermissionError> {
        Err(PermissionError::ClipboardDenied {
            plugin: self.plugin_id.clone(),
        })
    }

    /// Petition a provider registration (§25.4 registration-based
    /// extension points). On success the caller registers the provider
    /// under the `x.`-prefixed id returned (§10.2 registry rule).
    pub fn petition_registration(
        &self,
        provider_id: &str,
        cap: Capability,
    ) -> Result<String, PermissionError> {
        if !self.granted.has_capability(cap) {
            return Err(PermissionError::RegistrationDenied {
                plugin: self.plugin_id.clone(),
                provider_id: provider_id.to_string(),
                reason: RegistrationRefusal::CapabilityNotGranted(cap),
            });
        }
        if !self.manifest_provides.iter().any(|p| p == provider_id) {
            return Err(PermissionError::RegistrationDenied {
                plugin: self.plugin_id.clone(),
                provider_id: provider_id.to_string(),
                reason: RegistrationRefusal::NotInProvides,
            });
        }
        Ok(format!("x.{provider_id}"))
    }

    /// Petition a document mutation: the envelope must at least be a
    /// JSON object; everything further (schema, kind, params, stage-5
    /// `requires`) is the ENGINE's validation pipeline — one door, one
    /// truth (D-48).
    pub fn petition_apply<'a>(&self, envelope_json: &'a str) -> Result<&'a str, PermissionError> {
        match serde_json::from_str::<serde_json::Value>(envelope_json) {
            Ok(v) if v.is_object() => Ok(envelope_json),
            _ => Err(PermissionError::MalformedEnvelope {
                plugin: self.plugin_id.clone(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::permissions::Grant;
    use crate::plugin::session::SessionStore;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "appname-gate-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn manifest(perms: &str) -> Manifest {
        Manifest::parse(
            format!(
                r#"{{
                "schema": "appname.plugin.v1",
                "id": "org.example.t",
                "version": "1.0.0",
                "api": {{ "min": "1.0", "max": "1.x" }},
                "entry": "plugin.wasm",
                "permissions": {perms},
                "provides": [ {{ "type": "filter", "id": "org.example.t.f" }} ]
            }}"#
            )
            .as_bytes(),
        )
        .unwrap()
    }

    fn gate(perms: &str, grants: Vec<Grant>) -> Gatekeeper {
        let m = manifest(perms);
        let mut set: GrantSet = grants.into_iter().collect();
        set.grant_silent(&m.permissions);
        let session = SessionStore::new(tmp("session")).for_plugin(&m.id).unwrap();
        Gatekeeper::new(&m, set, session).0
    }

    #[test]
    fn session_store_reads_and_writes_flow() {
        let g = gate(r#"["filter.register"]"#, vec![]);
        let p = g.petition_fs("presets/a.json", true).unwrap();
        assert!(p.starts_with(g.session_root()));
    }

    #[test]
    fn ambient_fs_denied_typed_with_wire_id() {
        let g = gate(r#"["filter.register"]"#, vec![]);
        let err = g.petition_fs("/etc/passwd", false).unwrap_err();
        assert!(matches!(
            err,
            PermissionError::FsDenied { write: false, .. }
        ));
        assert_eq!(err.wire_id(), Some("error.permission.filesystem_denied"));
    }

    #[test]
    fn scoped_grant_admits_in_scope_only() {
        let granted_dir = tmp("granted");
        let g = gate(
            r#"["filter.register", "fs.read"]"#,
            vec![Grant::FsRead(granted_dir.clone())],
        );
        let inside = format!("{}/tex.png", granted_dir.display());
        assert!(g.petition_fs(&inside, false).is_ok());
        // Out of scope:
        assert!(g.petition_fs("/etc/passwd", false).is_err());
        // Right dir, wrong scope (write via a read grant):
        assert!(g.petition_fs(&inside, true).is_err());
    }

    #[test]
    fn net_denied_even_when_granted() {
        // The hostile-corpus pin: a rogue "net" grant in the state file
        // must NOT open the network in v1 (privacy hard-gate).
        let g = gate(r#"["filter.register", "net"]"#, vec![Grant::Net]);
        let err = g.petition_net().unwrap_err();
        assert!(matches!(err, PermissionError::NetDenied { .. }));
        assert_eq!(err.wire_id(), Some("error.permission.network_denied"));
    }

    #[test]
    fn clipboard_denied_in_v1() {
        let g = gate(
            r#"["filter.register", "clipboard"]"#,
            vec![Grant::Clipboard],
        );
        let err = g.petition_clipboard().unwrap_err();
        assert!(matches!(err, PermissionError::ClipboardDenied { .. }));
        // D2: clipboard denial MUST NOT borrow the network wire id — it
        // has none, and surfaces through the ordinary error path.
        assert_eq!(err.wire_id(), None);
        assert!(err.to_string().contains("clipboard"));
    }

    #[test]
    fn registration_gate_prefixes_and_refuses() {
        let g = gate(r#"["filter.register"]"#, vec![]);
        assert_eq!(
            g.petition_registration("org.example.t.f", Capability::FilterRegister)
                .unwrap(),
            "x.org.example.t.f"
        );
        // Not in provides:
        assert!(matches!(
            g.petition_registration("org.example.t.ghost", Capability::FilterRegister),
            Err(PermissionError::RegistrationDenied {
                reason: RegistrationRefusal::NotInProvides,
                ..
            })
        ));
        // Capability not granted (tool.register never requested):
        assert!(matches!(
            g.petition_registration("org.example.t.f", Capability::ToolRegister),
            Err(PermissionError::RegistrationDenied {
                reason: RegistrationRefusal::CapabilityNotGranted(Capability::ToolRegister),
                ..
            })
        ));
    }

    #[test]
    fn apply_passes_objects_and_refuses_garbage() {
        let g = gate(r#"["filter.register"]"#, vec![]);
        assert!(g
            .petition_apply(r#"{"schema":"appname.command.v1"}"#)
            .is_ok());
        assert!(g.petition_apply("not json").is_err());
        assert!(g.petition_apply("[1,2]").is_err());
    }
}
