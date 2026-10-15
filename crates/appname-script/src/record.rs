// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The action recorder — a JOURNAL-DELTA TAP over existing read surfaces.
//!
//! Spec: ES §26.1 "the recorder taps the command stream (§10) so anything
//! undoable is recordable" + recorder rules 1-4; D-48 (values captured —
//! the applied envelope IS the recorded step).
//!
//! # Recording seam (the honest design, lane decision)
//!
//! The recorder holds NO hook inside the engine and adds NO mutation
//! path: it marks the journal cursor at [`Recorder::begin`] and, at
//! [`Recorder::finish`], reads the in-effect journal delta
//! (`Engine::journal()[mark..cursor]`) — the same append-only envelope
//! stream the autosave log persists and replay consumes. Consequences,
//! all by construction rather than by filtering:
//!
//! * **Rule 1** (only document mutations): the journal contains nothing
//!   else — view/window/tool state never reaches it.
//! * **Rule 2** (committed parameters): an envelope enters the journal
//!   only on a successful apply; cancelled dialogs never applied.
//! * **Undo during recording**: the engine's linear history truncates
//!   undone tails on the next apply; a step undone and left undone sits
//!   past the cursor and is NOT recorded (the recording captures the
//!   net in-effect stream).
//! * **Rule 4** (coalesced strokes): a stroke is already one journal
//!   envelope referencing a `samples` asset.
//!
//! Rule 3 capture rides along: for every recorded step targeting a layer,
//! the layer's NAME and info-row tree index are captured into the step's
//! `recordedTarget` (lane extension, `action.rs` module docs), and every
//! asset the step references is harvested from the engine's asset store
//! into the action's asset table (content-addressed ids).
//!
//! An OUT-OF-PROCESS recorder (an ABI-only host such as the QML shell)
//! cannot use this seam today: `appname_doc_history_json` rows carry no
//! params by design. That missing read surface is a TYPED GAP recorded
//! for O-INT in the lane log — not worked around here.

use serde_json::Value;
use uuid::Uuid;

use appname_core::{Engine, EngineError, ErrorCode};

use crate::action::{Action, ActionAsset, CommandStep, RecordedTarget, Step};

/// A recording in progress. Purely observational: holds a journal mark,
/// never a mutable engine borrow.
#[derive(Debug, Clone)]
pub struct Recorder {
    name: String,
    mark: usize,
}

impl Recorder {
    /// Start recording: mark the current journal cursor.
    pub fn begin(engine: &Engine, name: &str) -> Recorder {
        Recorder {
            name: name.to_string(),
            mark: engine.journal_cursor(),
        }
    }

    /// The journal mark this recording started at.
    pub fn mark(&self) -> usize {
        self.mark
    }

    /// Stop recording and build the action from the in-effect journal
    /// delta. If undo carried the cursor BELOW the mark, the delta is
    /// empty (the recorded edits are no longer in effect) — an empty
    /// action with a warning is honest output, so this returns `Ok` with
    /// zero steps rather than an error.
    pub fn finish(self, engine: &Engine) -> Result<Action, EngineError> {
        let cursor = engine.journal_cursor();
        let lo = self.mark.min(cursor);
        let mut steps = Vec::with_capacity(cursor - lo);
        let mut assets: Vec<ActionAsset> = Vec::new();
        for env in &engine.journal()[lo..cursor] {
            let command = serde_json::to_value(env).map_err(|e| {
                EngineError::new(
                    ErrorCode::InvalidArg,
                    format!("recorded envelope failed to serialize: {e}"),
                )
            })?;
            let recorded_target = env
                .target
                .as_ref()
                .and_then(|t| t.layer_id)
                .and_then(|id| capture_layer(engine, id));
            for id in referenced_assets(&command) {
                if assets.iter().any(|a| a.id == id) {
                    continue;
                }
                if let (Some(bytes), Some(kind)) = (
                    engine.assets().bytes(id).map(<[u8]>::to_vec),
                    engine.assets().kind_of(id),
                ) {
                    assets.push(ActionAsset { id, kind, bytes });
                }
            }
            steps.push(Step::Command(CommandStep {
                command,
                enabled: true,
                parameterized: Vec::new(),
                recorded_target,
            }));
        }
        Ok(Action {
            id: Some(Uuid::new_v4()),
            name: self.name,
            steps,
            assets,
        })
    }
}

/// Rule-3 capture: name + info-row tree index for a live layer id. The
/// index mirrors `appname_doc_info_json`'s layer rows EXACTLY (position in
/// the document's z-order walk, enumerated before filtering) so recorded
/// indices and info indices are one vocabulary.
fn capture_layer(engine: &Engine, id: Uuid) -> Option<RecordedTarget> {
    let id = appname_core::graph::id::LayerId::from_uuid(id);
    let doc = engine.document()?;
    let (index, name) = doc
        .z_order_walk()
        .iter()
        .enumerate()
        .find(|(_, walk_id)| **walk_id == id)
        .and_then(|(i, walk_id)| doc.layer(*walk_id).map(|l| (i, l.name().to_string())))?;
    Some(RecordedTarget {
        layer_name: Some(name),
        tree_index: Some(index),
    })
}

/// Every asset id a recorded command references: the envelope-level
/// `samples` field (§10.2) plus any `assetId` string member anywhere in
/// `params` (the `edit.paste` carrier shape) — resolved ids only (the
/// caller checks the store).
fn referenced_assets(command: &Value) -> Vec<Uuid> {
    let mut out = Vec::new();
    if let Some(s) = command.get("samples").and_then(Value::as_str) {
        if let Ok(id) = Uuid::parse_str(s) {
            out.push(id);
        }
    }
    if let Some(params) = command.get("params") {
        collect_asset_ids(params, &mut out);
    }
    out
}

fn collect_asset_ids(v: &Value, out: &mut Vec<Uuid>) {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                if k == "assetId" {
                    if let Some(s) = val.as_str() {
                        if let Ok(id) = Uuid::parse_str(s) {
                            out.push(id);
                        }
                    }
                }
                collect_asset_ids(val, out);
            }
        }
        Value::Array(arr) => {
            for val in arr {
                collect_asset_ids(val, out);
            }
        }
        _ => {}
    }
}
