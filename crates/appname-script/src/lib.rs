// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-script` — actions/macros v1 (CP-0.7) and, at CP-1.0, the
//! sandboxed scripting hosts (wasm/lua).
//!
//! Spec: ES §26.1 (action format + recorder rules; the mirrored contract
//! is `contracts/schemas/action.v1.json`), §26.2 (batch + report +
//! template grammar), §25.3 (the LATER sandboxed tier — not in this
//! checkpoint); D-48 (values captured: replay never re-executes dynamics
//! — the recorded envelopes ARE the action); plan §2.4 rows
//! "Actions/macros v1" (lane O-SCRIPT).
//!
//! Determinism spine (D-48, ES §10): every document mutation is a
//! `appname.command.v1` envelope through the single apply entry — this
//! crate RECORDS the journal (a read surface), REPLAYS through the
//! ordinary apply door ([`replay::ApplyDoor`]), and adds no side-door
//! mutation API, ever.
//!
//! Module map:
//! * [`action`] — the `appname.action.v1` model (parse/validate/
//!   serialize) with the two log-noted schema-open lane extensions
//!   (rule-3 `recordedTarget` capture, rule-4 `assets` carriage).
//! * [`record`] — the journal-delta recorder (the recording seam:
//!   existing engine read surfaces only).
//! * [`replay`] — §26.1 playback over a caller-supplied apply door.
//! * [`template`] — the §26.2 output-template substitution grammar.
//! * [`batch`] — §26.2 batch jobs + `appname.batchreport.v1`
//!   (W20/W21 groundwork; the CLI `--batch` flag wiring is O-CORE's).
//! * [`plugin`] — the §25 SANDBOXED plugin tier v1 (CP-1.0): manifest,
//!   permissions/capability gates, session store, lifecycle, sandbox-
//!   host seam; wasm/lua bindings staged record-first (its module docs).

pub mod action;
pub mod batch;
pub mod plugin;
pub mod record;
pub mod replay;
pub mod template;

pub use action::{Action, ActionAsset, CommandStep, RecordedTarget, Step, StopStep, ACTION_SCHEMA};
pub use record::Recorder;
pub use replay::{
    replay, ApplyDoor, EngineDoor, ReplayOptions, ReplayOutcome, ReplayReport, StepReport,
    StepStatus,
};
