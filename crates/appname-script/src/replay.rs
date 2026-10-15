// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Action playback — §26.1 replay semantics over the ONE mutation door.
//!
//! Spec: ES §26.1 (recorder rules 3-4, step semantics), §26.2 (step
//! reports; `error.batch.step_failed`), D-48 (values captured: replay
//! never re-executes dynamics — the recorded envelopes ARE the action;
//! stroke steps re-render deterministically from their captured `samples`
//! assets inside the ordinary kind implementation).
//!
//! # The door ([`ApplyDoor`])
//!
//! Playback NEVER mutates an engine directly: every step goes through a
//! caller-supplied [`ApplyDoor`], whose baseline implementation
//! ([`EngineDoor`]) is exactly `Engine::apply_envelope` — the spine entry
//! (§10). Front ends with per-command side rails (the C ABI's event
//! emission + autosave binding) implement the trait over their own
//! pipeline so a replayed step is indistinguishable from a hand-applied
//! command. No side-door mutation exists here (preamble rule 5).
//!
//! # Replay semantics (pinned by tests)
//!
//! * Disabled steps are skipped without error (§26.1).
//! * A `stop` step is reported with its verbatim author message;
//!   `allowContinue: false` HALTS the replay (headless has no dialog —
//!   the halt is the → "Stop" button; W20 posture).
//! * `parameterized` pointers substitute from [`ReplayOptions::vars`]
//!   (keyed by the pointer string). A supplied pointer that does not
//!   resolve is a STEP ERROR (§26.1); a pointer with NO supplied variable
//!   keeps the recorded value (the headless equivalent of accepting the
//!   prompt's default — lane election, log-noted).
//! * Target rewriting per rule 3: a recorded `target.layerId` that no
//!   longer resolves is re-resolved by captured layer NAME first
//!   (depth-first), then by captured TREE INDEX; neither resolving is a
//!   step error. A symbolic `layerId: null` means "active layer at
//!   replay" and resolves via [`ReplayOptions::active_layer`] (headless
//!   callers pass one; absent = step error).
//! * Every replayed step is applied under a FRESHLY MINTED command id:
//!   ids are command identity, not captured values — the journal rejects
//!   duplicates, and one action must be re-runnable on the document that
//!   recorded it.
//! * CREATED-OBJECT ids re-mint the same way: recorded `new*Id` params
//!   (`newLayerId`, `newMaskId`, … — the §10.3 created-id convention)
//!   name objects that would collide when the action re-runs, so replay
//!   mints fresh ids and carries an old→new REMAP applied to every later
//!   step (an intra-action reference — set props on the layer this
//!   action just added — stays coherent, and rule-3 name resolution only
//!   answers for references to PRE-EXISTING objects). `newAssetId` is
//!   the one exclusion: asset ids are content-addressed and re-converge
//!   by re-ingestion, never by minting.
//! * A step error stops the replay (remaining steps unattempted); the
//!   report carries `error.batch.step_failed` + the engine's structured
//!   error, per the §26.2 report shape.

use serde_json::{json, Map, Value};
use uuid::Uuid;

use appname_core::engine::assets::AssetKind;
use appname_core::engine::envelope::Envelope;
use appname_core::engine::ApplyReceipt;
use appname_core::{Engine, EngineError, ErrorCode};

use crate::action::{pointer_set, Action, RecordedTarget, Step};

/// The one mutation seam playback uses (module docs).
pub trait ApplyDoor {
    /// Read access for target resolution between steps.
    fn engine(&self) -> &Engine;
    /// Apply one validated envelope through the caller's ordinary
    /// pipeline.
    fn apply(&mut self, env: Envelope) -> Result<ApplyReceipt, EngineError>;
    /// Ingest one captured asset blob (content-addressed; idempotent).
    fn ingest_asset(&mut self, kind: AssetKind, bytes: &[u8]) -> Result<Uuid, EngineError>;
}

/// The baseline door: the engine's own spine entries, nothing else.
pub struct EngineDoor<'a>(pub &'a mut Engine);

impl ApplyDoor for EngineDoor<'_> {
    fn engine(&self) -> &Engine {
        self.0
    }
    fn apply(&mut self, env: Envelope) -> Result<ApplyReceipt, EngineError> {
        self.0.apply_envelope(env)
    }
    fn ingest_asset(&mut self, kind: AssetKind, bytes: &[u8]) -> Result<Uuid, EngineError> {
        match kind {
            AssetKind::Samples => Ok(self.0.add_samples_asset(bytes.to_vec())),
            AssetKind::RasterImage => self.0.add_raster_asset(bytes.to_vec()),
            // MECHANICAL companion arms (core door slice 2, Doors 10 +
            // D-ML-1 — RATIFIED by O-SCRIPT, return leg,
            // reports/O-SCRIPT-log.md): §23.5 embedded faces +
            // appname.mask.v1 coverage planes.
            AssetKind::FontFace => self.0.add_font_asset(bytes.to_vec()),
            AssetKind::MaskPlane => self.0.add_mask_asset(bytes.to_vec()),
        }
    }
}

/// Replay-time inputs.
#[derive(Debug, Clone, Default)]
pub struct ReplayOptions {
    /// Batch variables, keyed by the parameterized POINTER string
    /// (`"/params/points"` → value). §26.2's `overrides`.
    pub vars: Map<String, Value>,
    /// The "active layer at replay time" a symbolic `layerId: null`
    /// resolves to (§26.1 rule 3). Headless callers supply one; `None`
    /// makes null-target steps a step error.
    pub active_layer: Option<Uuid>,
}

/// Per-step outcome (§26.2 report rows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepStatus {
    Ok,
    Skipped,
    Stop,
    Error,
}

impl StepStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            StepStatus::Ok => "ok",
            StepStatus::Skipped => "skipped",
            StepStatus::Stop => "stop",
            StepStatus::Error => "error",
        }
    }
}

/// One report row.
#[derive(Debug, Clone)]
pub struct StepReport {
    pub index: usize,
    /// Command kind (command steps only).
    pub kind: Option<String>,
    pub status: StepStatus,
    /// Stop steps: the author's verbatim message.
    pub message: Option<String>,
    /// Error steps: human-oriented failure detail.
    pub detail: Option<String>,
}

/// Whole-replay outcome.
#[derive(Debug, Clone)]
pub enum ReplayOutcome {
    /// Every enabled step ran (stops with `allowContinue` included).
    Completed,
    /// A `stop` with `allowContinue: false` ended the replay.
    Halted { at: usize },
    /// A step failed; `error` is the engine's structured error (or a
    /// replay-stage error in the same taxonomy).
    Failed { at: usize, error: EngineError },
}

/// The full replay report.
#[derive(Debug, Clone)]
pub struct ReplayReport {
    pub outcome: ReplayOutcome,
    pub steps: Vec<StepReport>,
}

impl ReplayReport {
    pub fn ok(&self) -> bool {
        !matches!(self.outcome, ReplayOutcome::Failed { .. })
    }

    /// Wire form: `{"status":"ok"|"halted"|"error","steps":[…],"error"?}`
    /// — the §26.2 per-file `steps` rows plus a status summary; the error
    /// object carries the NORMATIVE `error.batch.step_failed` message id.
    pub fn to_value(&self) -> Value {
        let steps: Vec<Value> = self
            .steps
            .iter()
            .map(|s| {
                let mut row = Map::new();
                row.insert("index".into(), json!(s.index));
                if let Some(k) = &s.kind {
                    row.insert("kind".into(), json!(k));
                }
                row.insert("status".into(), json!(s.status.as_str()));
                if let Some(m) = &s.message {
                    row.insert("message".into(), json!(m));
                }
                if let Some(d) = &s.detail {
                    row.insert("detail".into(), json!(d));
                }
                Value::Object(row)
            })
            .collect();
        let mut obj = Map::new();
        obj.insert(
            "status".into(),
            json!(match self.outcome {
                ReplayOutcome::Completed => "ok",
                ReplayOutcome::Halted { .. } => "halted",
                ReplayOutcome::Failed { .. } => "error",
            }),
        );
        obj.insert("steps".into(), Value::Array(steps));
        match &self.outcome {
            ReplayOutcome::Halted { at } => {
                obj.insert("haltedAt".into(), json!(at));
            }
            ReplayOutcome::Failed { at, error } => {
                obj.insert(
                    "error".into(),
                    json!({
                        "messageId": "error.batch.step_failed",
                        "step": at,
                        "code": error.code.as_str(),
                        "detail": error.message,
                    }),
                );
            }
            ReplayOutcome::Completed => {}
        }
        Value::Object(obj)
    }
}

/// Replay `action` through `door`. Never panics on malformed steps —
/// every failure is a typed report row.
pub fn replay(action: &Action, door: &mut dyn ApplyDoor, opts: &ReplayOptions) -> ReplayReport {
    let mut steps = Vec::with_capacity(action.steps.len());
    // Created-id remap (module docs): recorded `new*Id` → freshly minted,
    // applied to every subsequent step.
    let mut remap: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    // Asset re-entry FIRST (§9.6 rule 2: captured values precede the
    // envelopes that reference them). Content-addressed ids converge on
    // the recorded ids — verified so a divergence is typed, not silent.
    for asset in &action.assets {
        match door.ingest_asset(asset.kind, &asset.bytes) {
            Ok(id) if id == asset.id => {}
            Ok(id) => {
                return ReplayReport {
                    outcome: ReplayOutcome::Failed {
                        at: 0,
                        error: EngineError::new(
                            ErrorCode::InvalidArg,
                            format!(
                                "action asset {} re-ingested under a different id {} — \
                                 content addressing diverged",
                                asset.id, id
                            ),
                        ),
                    },
                    steps,
                };
            }
            Err(e) => {
                return ReplayReport {
                    outcome: ReplayOutcome::Failed { at: 0, error: e },
                    steps,
                };
            }
        }
    }

    for (index, step) in action.steps.iter().enumerate() {
        match step {
            Step::Stop(stop) => {
                if !stop.enabled {
                    steps.push(StepReport {
                        index,
                        kind: None,
                        status: StepStatus::Skipped,
                        message: None,
                        detail: None,
                    });
                    continue;
                }
                steps.push(StepReport {
                    index,
                    kind: None,
                    status: StepStatus::Stop,
                    message: Some(stop.message.clone()),
                    detail: None,
                });
                if !stop.allow_continue {
                    return ReplayReport {
                        outcome: ReplayOutcome::Halted { at: index },
                        steps,
                    };
                }
            }
            Step::Command(cstep) => {
                let kind = cstep
                    .command
                    .get("kind")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if !cstep.enabled {
                    steps.push(StepReport {
                        index,
                        kind,
                        status: StepStatus::Skipped,
                        message: None,
                        detail: None,
                    });
                    continue;
                }
                match replay_command(cstep, door, opts, &mut remap) {
                    Ok(()) => steps.push(StepReport {
                        index,
                        kind,
                        status: StepStatus::Ok,
                        message: None,
                        detail: None,
                    }),
                    Err(error) => {
                        steps.push(StepReport {
                            index,
                            kind,
                            status: StepStatus::Error,
                            message: None,
                            detail: Some(format!("{}: {}", error.code.as_str(), error.message)),
                        });
                        return ReplayReport {
                            outcome: ReplayOutcome::Failed { at: index, error },
                            steps,
                        };
                    }
                }
            }
        }
    }
    ReplayReport {
        outcome: ReplayOutcome::Completed,
        steps,
    }
}

fn step_err(msg: String) -> EngineError {
    EngineError::new(ErrorCode::BadTarget, msg)
}

fn replay_command(
    cstep: &crate::action::CommandStep,
    door: &mut dyn ApplyDoor,
    opts: &ReplayOptions,
    remap: &mut std::collections::HashMap<String, String>,
) -> Result<(), EngineError> {
    let mut cmd = cstep.command.clone();

    // Parameterized substitution (module docs): supplied vars replace the
    // recorded value at each pointer; an unresolvable SUPPLIED pointer is
    // a step error; an unsupplied pointer keeps the recorded value.
    for pointer in &cstep.parameterized {
        if let Some(v) = opts.vars.get(pointer) {
            pointer_set(&mut cmd, pointer, v.clone())
                .map_err(|m| EngineError::new(ErrorCode::InvalidArg, m))?;
        }
    }

    // Created-id coherence (module docs): earlier steps' remapped ids
    // first, then mint fresh ids for THIS step's `new*Id` params.
    apply_remap(&mut cmd, remap);
    mint_created_ids(&mut cmd, remap);

    // Rule-3 target rewriting.
    rewrite_target(
        &mut cmd,
        cstep.recorded_target.as_ref(),
        door.engine(),
        opts.active_layer,
    )?;

    // Fresh command identity + schema const; then the ORDINARY envelope
    // door (stages 0-7) — nothing about a replayed command is special.
    if let Value::Object(map) = &mut cmd {
        map.insert("id".into(), json!(Uuid::new_v4().to_string()));
        map.entry("schema".to_string())
            .or_insert_with(|| json!(crate::action::envelope_schema()));
    }
    let env = Envelope::from_value(cmd)?;
    door.apply(env).map(|_| ())
}

/// Replace every string equal to a remapped id, anywhere in the command
/// (targets and params — a later step may reference a created object from
/// either position).
fn apply_remap(v: &mut Value, remap: &std::collections::HashMap<String, String>) {
    if remap.is_empty() {
        return;
    }
    match v {
        Value::String(s) => {
            if let Some(new) = remap.get(s.as_str()) {
                *s = new.clone();
            }
        }
        Value::Object(map) => {
            for (_, val) in map.iter_mut() {
                apply_remap(val, remap);
            }
        }
        Value::Array(arr) => {
            for val in arr.iter_mut() {
                apply_remap(val, remap);
            }
        }
        _ => {}
    }
}

/// Mint fresh ids for this step's created-object params: object members
/// named `new<Upper>…Id` holding a UUID string (`newLayerId`,
/// `newMaskId`, `newGroupId`, …), EXCEPT `newAssetId` (content-addressed,
/// module docs). The old→new pair joins the remap for later steps.
fn mint_created_ids(v: &mut Value, remap: &mut std::collections::HashMap<String, String>) {
    match v {
        Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                let is_created_id = k.len() > 5
                    && k.starts_with("new")
                    && k.ends_with("Id")
                    && k.as_bytes()[3].is_ascii_uppercase()
                    && k != "newAssetId";
                if is_created_id {
                    if let Value::String(s) = val {
                        if Uuid::parse_str(s).is_ok() {
                            let fresh = Uuid::new_v4().to_string();
                            remap.insert(s.clone(), fresh.clone());
                            *s = fresh;
                            continue;
                        }
                    }
                }
                mint_created_ids(val, remap);
            }
        }
        Value::Array(arr) => {
            for val in arr.iter_mut() {
                mint_created_ids(val, remap);
            }
        }
        _ => {}
    }
}

/// §26.1 rule 3. Only `target.layerId` is rewritten at this checkpoint;
/// mask/channel ids pass through to the engine's own stage-4 resolution
/// (typed `error.command.bad_target` when stale — log-noted scope).
fn rewrite_target(
    cmd: &mut Value,
    recorded: Option<&RecordedTarget>,
    engine: &Engine,
    active: Option<Uuid>,
) -> Result<(), EngineError> {
    let Some(target) = cmd.get_mut("target") else {
        return Ok(());
    };
    let Value::Object(target) = target else {
        return Ok(()); // malformed target: the envelope door answers.
    };
    let Some(layer_id) = target.get("layerId") else {
        return Ok(());
    };
    let new_value = match layer_id {
        Value::Null => match active {
            // Symbolic "active layer at replay time".
            Some(a) => json!(a.to_string()),
            None => {
                return Err(step_err(
                    "step targets the active layer (layerId null) and no active \
                     layer was supplied to this headless replay"
                        .into(),
                ))
            }
        },
        Value::String(s) => {
            let Ok(id) = Uuid::parse_str(s) else {
                return Ok(()); // not a UUID: the envelope door answers.
            };
            if layer_exists(engine, id) {
                return Ok(()); // recorded id still resolves — keep it.
            }
            let by_name = recorded
                .and_then(|r| r.layer_name.as_deref())
                .and_then(|name| find_layer_by_name(engine, name));
            let resolved = by_name.or_else(|| {
                recorded
                    .and_then(|r| r.tree_index)
                    .and_then(|i| layer_at_walk_index(engine, i))
            });
            match resolved {
                Some(id) => json!(id.to_string()),
                None => {
                    let named = recorded
                        .and_then(|r| r.layer_name.as_deref())
                        .unwrap_or("<unrecorded>");
                    return Err(step_err(format!(
                        "layer {named:?} not found by name or index (recorded id {s})"
                    )));
                }
            }
        }
        _ => return Ok(()),
    };
    target.insert("layerId".into(), new_value);
    Ok(())
}

fn layer_exists(engine: &Engine, id: Uuid) -> bool {
    let lid = appname_core::graph::id::LayerId::from_uuid(id);
    engine
        .document()
        .map(|d| d.layer(lid).is_some())
        .unwrap_or(false)
}

/// Depth-first (z-order-walk order) FIRST match by name — rule 3.
fn find_layer_by_name(engine: &Engine, name: &str) -> Option<Uuid> {
    let doc = engine.document()?;
    doc.z_order_walk()
        .iter()
        .find(|id| doc.layer(**id).map(|l| l.name() == name).unwrap_or(false))
        .map(|id| id.as_uuid())
}

/// The recorded tree index, interpreted in the SAME vocabulary the
/// recorder captured it in (position in the z-order walk / info rows).
fn layer_at_walk_index(engine: &Engine, index: usize) -> Option<Uuid> {
    let doc = engine.document()?;
    let walk = doc.z_order_walk();
    let id = walk.get(index)?;
    doc.layer(*id).map(|_| id.as_uuid())
}
