// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname` — the headless CLI (plan §2.1 row "Headless CLI v1", ported
//! from the proto CLI; OQ-4: the binary name is the proto's `appname`).
//! Every mutation is an `appname.command.v1` envelope fed to the engine as
//! RAW text (WD-1) through the one apply entry — exactly the path every
//! other front end uses (D-48).
//!
//! Machine contract (proto-pinned, REPORT-A D-series):
//! - exit 0 = ok, 1 = command rejected, 2 = IO error (`error.io.*`); clap
//!   usage errors also exit 2 (clap's default);
//! - `--json` prints exactly one JSON object on stdout:
//!   `{"ok":true, ...}` or `{"ok":false, "error":{code,message,field?}}`
//!   (+ `id`/`line` context when known);
//! - human-readable strings go through the i18n catalog (`--lang` >
//!   `APPNAME_LANG` > en; key-echo fallback); wire constants (kinds, codes,
//!   PASS/FAIL) are never translated.
//!
//! CP-0.1 divergences from the proto surface (lane log): `new --background`
//! takes the doc.new contract enum (not `#RRGGBB`); `export` serves the
//! six §29.1 formats (png byte-stable core path; jpeg/webp/tiff/gif/bmp
//! through the merged io export pipeline, finding 4); `selftest` is
//! the self-contained determinism proof (goldens are O-QA's lane); undo and
//! redo run replay-as-recovery gated on `state_hash` equality (stronger
//! than the proto's structural gate — tiles round-trip losslessly).

mod i18n;
mod selftest;
// O-TOOLS lifted the stroke KindImpls home (byte-identical); the alias
// keeps every call site (incl. selftest's super:: paths) unchanged.
use appname_tools::kind_impls as stroke_kinds;

/// The CLI engine assembly: THE production constructor (core door slice
/// 2, item D — the single-source assembly truth). The chain itself lives
/// in `appname_assembly::PRODUCTION_CHAIN`; a new registration is one
/// manifest row there and reaches every shipped surface at once. The
/// crate's parity test enumerates this call site.
fn full_assembly() -> appname_core::EngineAssembly {
    appname_assembly::production_assembly()
}

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use appname_core::{
    load_package_with, save_package, ApplyReceipt, Engine, EngineError, ErrorCode, LoadedPackage,
};
use clap::{ArgGroup, Parser, Subcommand};
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "appname",
    version,
    about = "Appname headless CLI: every edit is an appname.command.v1 envelope through one apply entry"
)]
struct Cli {
    /// Language for human-readable output (i18n/<code>.json; flag wins over
    /// the APPNAME_LANG env var, default en; key-echo fallback)
    #[arg(long, global = true, value_name = "CODE")]
    lang: Option<String>,
    /// Machine-readable JSON on stdout (one object per invocation)
    #[arg(long, global = true)]
    json: bool,
    /// §10.4 in-process history byte budget in MiB (cold undo deltas
    /// compress + spill to scratch over it, oldest steps drop at the
    /// newest-8 floor; default unbounded). The APPNAME_HISTORY_BUDGET_MB
    /// env var supplies a default when the flag is absent.
    #[arg(long, global = true, value_name = "MIB")]
    history_budget: Option<u64>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new document package (applies doc.new through the one entry)
    New {
        #[arg(long)]
        width: u32,
        #[arg(long)]
        height: u32,
        /// doc.new contract enum: transparent|white|black|background_color
        #[arg(long, default_value = "white")]
        background: String,
        #[arg(long, value_name = "DOC")]
        out: PathBuf,
    },
    /// Apply ONE appname.command.v1 envelope to a document
    #[command(group(ArgGroup::new("src").required(true).args(["command", "file"])))]
    Apply {
        #[arg(long)]
        doc: PathBuf,
        /// The envelope JSON, inline
        #[arg(long, value_name = "JSON")]
        command: Option<String>,
        /// Path to a file holding one envelope JSON
        #[arg(long, value_name = "CMD_JSON")]
        file: Option<PathBuf>,
    },
    /// Apply a JSONL script of envelopes (transactional: stop-on-error with
    /// line number, nothing persisted on failure)
    Script {
        #[arg(long)]
        doc: PathBuf,
        /// JSONL file: one envelope per line; blank lines and #-comments skipped
        #[arg(long, value_name = "OPS_JSONL")]
        file: PathBuf,
        /// Append each applied envelope to a §7.8 autosave journal (durable
        /// per command); a hard kill mid-script recovers via `recover`
        #[arg(long, value_name = "LOG")]
        autosave: Option<PathBuf>,
    },
    /// doc.duplicate (§10.3 R-dagger): deep-copy the document into a new
    /// package — no journal record on the source; the duplicate opens
    /// with empty in-process history at its own journal base
    Duplicate {
        #[arg(long)]
        doc: PathBuf,
        /// Where to write the duplicated package
        #[arg(long)]
        out: PathBuf,
    },
    /// §10.6 crash recovery: replay an autosave journal on top of its base
    /// package and write the recovered document
    Recover {
        /// The base .appname package the journal's BASE record anchors to
        #[arg(long)]
        doc: PathBuf,
        /// The autosave journal written by `script --autosave`
        #[arg(long, value_name = "LOG")]
        log: PathBuf,
        /// Where to write the recovered package
        #[arg(long)]
        out: PathBuf,
    },
    /// Undo the newest in-effect command(s) (replay-as-recovery across the
    /// package boundary)
    Undo {
        #[arg(long)]
        doc: PathBuf,
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
        steps: u32,
    },
    /// Redo previously undone command(s)
    Redo {
        #[arg(long)]
        doc: PathBuf,
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
        steps: u32,
    },
    /// List the journal as a history table
    History {
        #[arg(long)]
        doc: PathBuf,
    },
    /// Export the flattened composite
    Export {
        #[arg(long)]
        doc: PathBuf,
        #[arg(long, value_parser = ["png", "jpeg", "webp", "tiff", "gif", "bmp"])]
        format: String,
        #[arg(long)]
        out: PathBuf,
        /// Encoder quality 1..=100 (jpeg/webp lossy only)
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=100))]
        quality: Option<u8>,
    },
    /// Import a PNG as a new layer (an IO call, outside the journal)
    Import {
        #[arg(long)]
        doc: PathBuf,
        #[arg(long)]
        image: PathBuf,
        /// Layer name (default: the image file stem)
        #[arg(long, value_name = "NAME")]
        as_layer: Option<String>,
    },
    /// Document summary: size, layers table, selection state, journal length
    Info {
        #[arg(long)]
        doc: PathBuf,
    },
    /// Package asset operations (samples blobs for stroke envelopes)
    Asset {
        #[command(subcommand)]
        cmd: AssetCmd,
    },
    /// Run the CP-0.1 determinism self-check: script → save → reopen →
    /// replay → identical state hash, and byte-identical re-export across
    /// two independent runs
    Selftest {
        /// Keep the work directory for inspection
        #[arg(long)]
        keep: bool,
    },
}

#[derive(Subcommand)]
enum AssetCmd {
    /// Register an appname.samples.v1 blob as a package asset; prints its
    /// content-derived assetId (deterministic from the bytes — script
    /// authors can precompute it, so scripts can reference the id before
    /// this verb runs in a pipeline)
    Add {
        #[arg(long)]
        doc: PathBuf,
        /// Path to the raw samples blob (appname.samples.v1 bytes)
        #[arg(long, value_name = "BLOB")]
        file: PathBuf,
    },
    /// Register an encoded raster image (PNG) as an INGESTED raster asset —
    /// the carrier edit.paste/edit.paste_into envelopes reference via
    /// params.assetId (§9.2/§10.3; decode-validated at ingestion)
    AddImage {
        #[arg(long)]
        doc: PathBuf,
        /// Path to the encoded image (PNG at CP-0.5)
        #[arg(long, value_name = "IMAGE")]
        file: PathBuf,
    },
}

/// A terminal failure: exit class + structured error + quoting context.
struct Failure {
    exit: u8,
    envelope_id: Option<String>,
    line: Option<usize>,
    error: Value,
}

impl Failure {
    fn from_engine(e: EngineError) -> Failure {
        let exit = if e.code.as_str().starts_with("error.io.") {
            2
        } else {
            1
        };
        Failure {
            exit,
            envelope_id: None,
            line: None,
            error: serde_json::to_value(&e).expect("EngineError serializes"),
        }
    }

    fn with_id(mut self, id: Option<String>) -> Failure {
        self.envelope_id = id;
        self
    }

    fn io_read(context: &str, e: impl std::fmt::Display) -> Failure {
        Failure {
            exit: 2,
            envelope_id: None,
            line: None,
            error: json!({ "code": "error.io.read_failed", "message": format!("{context}: {e}") }),
        }
    }

    fn rejected(code: &str, message: String) -> Failure {
        Failure {
            exit: 1,
            envelope_id: None,
            line: None,
            error: json!({ "code": code, "message": message }),
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let lang = i18n::resolve_lang(cli.lang.as_deref());
    let cat = i18n::Catalog::load(&lang);
    match run(&cli, &cat) {
        Ok(code) => code,
        Err(f) => {
            if cli.json {
                let mut obj = json!({ "ok": false, "error": f.error });
                if let Some(id) = &f.envelope_id {
                    obj["id"] = json!(id);
                }
                if let Some(line) = f.line {
                    obj["line"] = json!(line);
                }
                println!("{obj}");
            } else {
                let code = f
                    .error
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    .to_string();
                let message = f
                    .error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let mut args: Vec<(&str, String)> = vec![("code", code), ("message", message)];
                if let Some(field) = f.error.get("field").and_then(Value::as_str) {
                    args.push(("field", field.to_string()));
                }
                if let Some(id) = &f.envelope_id {
                    args.push(("id", id.clone()));
                }
                if let Some(line) = f.line {
                    args.push(("line", line.to_string()));
                }
                eprintln!("{}", cat.tr("cli.error", &args));
            }
            ExitCode::from(f.exit)
        }
    }
}

/// Apply the global engine tuning flags to a freshly built/loaded engine.
fn tune_engine(cli: &Cli, engine: &mut Engine) {
    let mib = cli.history_budget.or_else(|| {
        std::env::var("APPNAME_HISTORY_BUDGET_MB")
            .ok()
            .and_then(|v| v.parse().ok())
    });
    if let Some(mib) = mib {
        engine.set_history_limits((mib as usize).saturating_mul(1024 * 1024), usize::MAX);
    }
}

fn load_doc(path: &Path) -> Result<LoadedPackage, Failure> {
    // Wave-2 arbitration 1: the CLI assembles the full engine (stroke +
    // core kinds) on every load, so reopened documents apply and replay
    // assembled kinds identically.
    load_package_with(&full_assembly(), path).map_err(Failure::from_engine)
}

fn save_doc(engine: &Engine, path: &Path) -> Result<(), Failure> {
    save_package(engine, path).map_err(Failure::from_engine)
}

/// Export dispatch — CLI-EXPORT ROUTING CLOSURE (the CP-0.7 recorded
/// residual, INTEGRATION-log render-review entry): parametric and
/// multi-raster documents now route through
/// `appname_render::composite_document_with` — THE COMPOSITOR TRUTH (the
/// §34.3 CPU oracle) — with the engine's op registry aboard, so the
/// strict-subset asymmetry collapses to equality at this door: a
/// §4.5-faded smart doc that RENDERS now EXPORTS identically (byte-for-
/// byte, pinned) instead of refusing at core's narrower arm.
///
/// Single-raster documents keep core's §4.4 reference path byte-for-byte
/// (the goldens bar); core's arm as a whole STAYS the reference
/// implementation for the render-vs-core cross-checks (asymmetry review:
/// bit-strict on the intersection — no reachable doc where core answers
/// pixels and render disagrees, so every previously-served export keeps
/// its bytes). Anything neither path can represent still refuses TYPED
/// (QA-F8 discipline: silent omission at a composite door is banned).
/// Parametric shape/text now SERVE through render (the composite-legs
/// slice: render gained the §20.2 fill / §23.2 text arms consuming core's
/// resolved page) — `has_visible_parametric_content` routes them to the
/// render path below, so a shape/text doc exports through the same
/// compositor truth the canvas readback reads.
fn export_bytes(engine: &Engine) -> Result<Vec<u8>, Failure> {
    let (w, h, plane) = export_plane(engine)?;
    appname_core::encode_composite_png(w, h, &plane).map_err(Failure::from_engine)
}

/// The flattened composite as a LINEAR PREMULTIPLIED RGBA f32 canvas
/// plane — the [`export_bytes`] routing (its doc above), returning the
/// plane instead of PNG bytes so the six-format io export pipeline
/// (finding 4) encodes the SAME pixels the CLI PNG path serves. PNG
/// stays byte-stable: it re-encodes this plane through the exact
/// `encode_composite_png` it always used.
fn export_plane(engine: &Engine) -> Result<(u32, u32, Vec<f32>), Failure> {
    let doc = engine.document().ok_or_else(|| {
        Failure::from_engine(EngineError::bad_target("package holds no document"))
    })?;
    let parametric = appname_core::has_visible_parametric_content(doc);
    if !parametric {
        let rasters = doc
            .z_order_walk()
            .iter()
            .filter_map(|id| doc.layer(*id))
            .filter(|n| n.pixels().is_some())
            .count();
        if rasters <= 1 {
            // Core's §4.4 reference path (goldens bar) — the SAME plane
            // export_png_bytes flattens (composite_plane_f32 internally).
            return appname_core::composite_plane_f32(engine).map_err(Failure::from_engine);
        }
    }
    // §19.1 one compositing truth: frame on `image_rect()` (the crop
    // sub-region while a crop is active, else the full canvas), identically
    // to core's reference door and the cabi readback/export.
    let frame = doc.geometry().image_rect();
    let composite = appname_render::composite_document_with(doc, engine.assembly(), frame)
        .map_err(|e| {
            Failure::from_engine(EngineError::new(
                ErrorCode::NotImplemented,
                format!("render composite refused: {e}"),
            ))
        })?;
    let plane = composite.region_rgba_f32(frame);
    Ok((frame.width() as u32, frame.height() as u32, plane))
}

/// Encode the flattened composite in a non-PNG format through the merged
/// §29.1 io export pipeline (finding 4: closes W21/W10/W13/W15 CLI legs).
/// PNG keeps its own byte-stable path ([`export_bytes`]); the other five
/// formats + jpeg/webp `--quality` route here.
fn export_bytes_format(
    engine: &Engine,
    format: &str,
    quality: Option<u8>,
) -> Result<Vec<u8>, Failure> {
    use appname_io::export::pipeline::encode_variant;
    use appname_io::export::preset::ExportPreset;
    use appname_io::export::ExportSource;

    let (w, h, plane) = export_plane(engine)?;
    let doc = engine
        .document()
        .expect("export_plane validated the document");
    let ppi = doc.geometry().resolution_ppi;

    // Build the preset: per-format default, layered with --quality for
    // the lossy formats (from_wire is the one option-parse path).
    let mut wire = serde_json::json!({ "format": format });
    if let Some(q) = quality {
        if format == "jpeg" || format == "webp" {
            wire["options"] = serde_json::json!({ "quality": q });
        } else {
            return Err(Failure::rejected(
                "error.api.invalid_arg",
                format!("--quality applies to jpeg/webp, not {format}"),
            ));
        }
    }
    let preset = ExportPreset::from_wire(&wire).map_err(codec_failure)?;
    let meta = appname_io::Metadata::default();
    let stem = "composite";
    let source = ExportSource {
        width: w,
        height: h,
        plane: &plane,
        icc: None,
        meta: &meta,
        doc_name: stem,
        ppi,
    };
    let mut cms = appname_color::Cms::new();
    let variant = encode_variant(&source, None, &preset, 1.0, &mut cms).map_err(codec_failure)?;
    Ok(variant.bytes)
}

/// Map an io `CodecError` onto the CLI's structured failure surface
/// (its own §9.5 wire code + message).
fn codec_failure(e: appname_io::CodecError) -> Failure {
    Failure::rejected(e.message_id(), e.to_string())
}

/// Best-effort envelope id extraction from raw JSON (for quoting in errors
/// when full validation failed).
fn sniff_id(text: &str) -> Option<String> {
    serde_json::from_str::<Value>(text)
        .ok()?
        .get("id")?
        .as_str()
        .map(str::to_string)
}

/// Best-effort envelope `kind` extraction from raw JSON (for the one-shot
/// apply-door selection guard below; full validation still happens in the
/// engine).
fn sniff_kind(text: &str) -> Option<String> {
    serde_json::from_str::<Value>(text)
        .ok()?
        .get("kind")?
        .as_str()
        .map(str::to_string)
}

/// Aud-parity **P-6** hazard set: command kinds whose contract
/// `selectionBehavior` GATES coverage on the active selection with a
/// SILENT whole-layer / whole-content fallback when no selection is active
/// (ES §13.1/§40.5). Kinds that instead HARD-FAIL a no-selection apply
/// with `error.command.selection_required` (content_aware_fill,
/// edit.paste_into, selection.transform/modify/save, layer.align
/// to:'selection', mask.add/channel.add/op.set_mask from:'selection',
/// stroke_path source:'selection', …) are deliberately EXCLUDED — they
/// already refuse loudly, so the composition is never silently destructive.
///
/// This list is sourced from the frozen contract mirror's
/// `selectionBehavior` field: every entry's contract says "gates coverage"
/// (plus `edit.cut`, whose "defines region … no selection = layer bounds
/// (whole content clears)" is the same silent-whole-content hazard). The
/// cross-check test `selection_scoped_set_matches_contracts` re-derives the
/// set from `contracts/commands/` and fails if this list drifts, so a
/// bogos-side contract change is caught here rather than silently missed.
/// (Fold-in candidate: this classification could live as a generated field
/// on the command registry, re-derived from the same `selectionBehavior`
/// source — recorded, not blocking.)
const SELECTION_SCOPED_KINDS: &[&str] = &[
    "blur_sharpen.stroke",
    "brush.stroke",
    "channel.apply_image",
    "channel.edit",
    "clone.stroke",
    "dodge_burn.stroke",
    "edit.cut",
    "eraser.stroke",
    "fill",
    "filter.apply",
    "heal.stroke",
    "image.auto_color",
    "image.auto_contrast",
    "image.auto_tone",
    "mask.edit",
    "smudge.stroke",
];

fn is_selection_scoped(kind: &str) -> bool {
    SELECTION_SCOPED_KINDS.contains(&kind)
}

/// The one-shot apply-door guard decision (P-6). Returns the loud warning
/// line to emit on stderr iff a selection-consuming `kind` is about to run
/// with NO active selection — i.e. the composition silently applies to the
/// whole layer. Pure so both arms are directly unit-testable.
///
/// The warning text is a plain stderr note for now; if it must become a
/// spec-defined MessageId (ES §27 / Appendix G), that is a fold-in — see
/// the lane report. (Kept OUT of the i18n catalog deliberately so it is not
/// an unregistered MessageId emission.)
fn selection_scoped_warning(kind: &str, selection_active: bool) -> Option<String> {
    if is_selection_scoped(kind) && !selection_active {
        Some(format!(
            "warning: '{kind}' scopes to the active selection, but this one-shot CLI \
             invocation has no active selection; one-shot mode does not persist selection \
             across invocations, so the operation applies to the WHOLE layer. Use `script` \
             to keep a selection live across a select-then-apply sequence."
        ))
    } else {
        None
    }
}

fn emit(cli: &Cli, machine: Value, human: Vec<String>) {
    if cli.json {
        println!("{machine}");
    } else {
        for line in human {
            println!("{line}");
        }
    }
}

fn receipt_json(receipt: &ApplyReceipt) -> Value {
    serde_json::to_value(receipt).expect("receipt serializes")
}

/// Replay-as-recovery for cross-invocation undo/redo (proto `ops` port):
/// rebuild by replaying the FULL journal, rewind to the persisted cursor,
/// gate on `state_hash` equality with the loaded document (imports outside
/// the journal fail this gate — proto divergence table), then move `steps`
/// (clamped) in the requested direction.
fn undo_redo(loaded: &Engine, steps: usize, redo: bool) -> Result<(Engine, usize), EngineError> {
    let len = loaded.journal().len();
    let cursor = loaded.journal_cursor();
    let available = if redo {
        len - cursor
    } else {
        cursor.saturating_sub(1)
    };
    if available == 0 {
        return Err(if redo {
            EngineError::new(ErrorCode::NothingToRedo, "nothing to redo")
        } else if cursor <= 1 {
            EngineError::new(
                ErrorCode::NothingToUndo,
                "nothing to undo (doc.new cannot be undone across the package boundary: \
                 an uninitialized document has no package form)",
            )
        } else {
            EngineError::new(ErrorCode::NothingToUndo, "nothing to undo")
        });
    }

    let replay_unavailable = |detail: String| {
        EngineError::new(
            ErrorCode::BadTarget,
            format!(
                "undo/redo unavailable for this package: its state cannot be rebuilt by \
                 replaying the journal ({detail}); common causes are a journal entry that \
                 references session-only state the package does not carry, or a package \
                 modified outside appname (replay-as-recovery)"
            ),
        )
    };

    // Rebuild the whole journal from doc.new through the SAME assembly and
    // assets (validates every entry; D-48).
    let mut engine = loaded.fresh_like();
    for (i, env) in loaded.journal().iter().enumerate() {
        engine.apply_envelope(env.clone()).map_err(|e| {
            replay_unavailable(format!(
                "journal entry {i} ({}) failed to re-apply: {e}",
                env.kind
            ))
        })?;
    }
    // Rewind to the persisted cursor and gate on byte-level fidelity.
    while engine.journal_cursor() > cursor {
        engine
            .undo()
            .map_err(|e| replay_unavailable(format!("rewinding to the persisted cursor: {e}")))?;
    }
    // §14.2 posture (wave-4 finding a): the live selection is session
    // state — a replayed journal may legitimately hold one the loaded
    // package cannot carry. The gate compares PERSISTENT state.
    if engine.state_hash_persistent() != loaded.state_hash_persistent() {
        return Err(replay_unavailable(
            "the replayed journal does not reproduce the persisted document state".to_string(),
        ));
    }
    // Move by UNDO-STACK STEPS, never by journal-index arithmetic (Q-9).
    //
    // ROOT CAUSE this replaces: `target = cursor - steps` counted † MARKER
    // entries as undoable units. A marker occupies a journal slot and NO
    // stack slot (§40.1): `Engine::undo` scans down past markers, and
    // `Engine::redo` ends with an absorb loop so a tail marker never
    // strands the cursor below it. With a marker newest, `cursor - 1`
    // therefore names a position the engine DELIBERATELY cannot occupy —
    // the two convergence loops undid a real step and then redid it,
    // landing back where they started. `performed` was computed BEFORE the
    // move and returned regardless, so the door reported ok:true with an
    // explicit FALSE COUNT while cursor, active flags, pixels and
    // redo-availability were all unchanged.
    //
    // The engine was right; only this arithmetic was wrong. Drive it one
    // step at a time and report WHAT ACTUALLY MOVED.
    let mut performed = 0usize;
    for _ in 0..steps {
        let before = engine.journal_cursor();
        let outcome = if redo { engine.redo() } else { engine.undo() };
        match outcome {
            Ok(_) => {
                // A step reporting success without moving the cursor IS the
                // Q-9 shape. Never count it.
                if engine.journal_cursor() == before {
                    break;
                }
                performed += 1;
            }
            // Exhausted part-way through: report the steps that did land.
            Err(_) if performed > 0 => break,
            // Nothing moved at all — surface the engine's typed refusal
            // rather than manufacturing a successful no-op.
            Err(e) => return Err(e),
        }
    }
    Ok((engine, performed))
}

fn run(cli: &Cli, cat: &i18n::Catalog) -> Result<ExitCode, Failure> {
    match &cli.cmd {
        Cmd::New {
            width,
            height,
            background,
            out,
        } => {
            let envelope_id = Uuid::new_v4();
            let text = json!({
                "schema": "appname.command.v1",
                "id": envelope_id,
                "kind": "doc.new",
                "params": { "width": width, "height": height, "background": background },
            })
            .to_string();
            let mut engine = full_assembly().build();
            tune_engine(cli, &mut engine);
            let receipt = engine
                .apply_json(&text)
                .map_err(|e| Failure::from_engine(e).with_id(Some(envelope_id.to_string())))?;
            save_doc(&engine, out)?;
            let doc_id = engine.document().expect("initialized").id();
            emit(
                cli,
                json!({ "ok": true, "out": out.display().to_string(), "docId": doc_id,
                        "envelopeId": envelope_id, "receipt": receipt_json(&receipt) }),
                vec![cat.tr(
                    "cli.new.ok",
                    &[
                        ("out", out.display().to_string()),
                        ("width", width.to_string()),
                        ("height", height.to_string()),
                        ("docId", doc_id.to_string()),
                        (
                            "layerId",
                            receipt
                                .created_ids
                                .last()
                                .map(Uuid::to_string)
                                .unwrap_or_default(),
                        ),
                    ],
                )],
            );
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Apply { doc, command, file } => {
            let text = match (command, file) {
                (Some(t), None) => t.clone(),
                (None, Some(p)) => std::fs::read_to_string(p)
                    .map_err(|e| Failure::io_read(&format!("reading {}", p.display()), e))?,
                _ => unreachable!("clap group enforces exactly one source"),
            };
            let mut loaded = load_doc(doc)?;
            tune_engine(cli, &mut loaded.engine);
            // Aud-parity P-6 guard: the one-shot `apply` door loads a fresh
            // engine per invocation, so the session-only selection (never
            // persisted — see appname_core mask.rs) is inactive here. A
            // selection-consuming kind therefore silently applies to the
            // WHOLE layer. Warn LOUDLY to stderr BEFORE applying (the op is
            // legitimate — do NOT refuse; the stdout `--json` contract stays
            // clean). The `script` door persists session state within its
            // run and is intentionally unaffected.
            if let Some(kind) = sniff_kind(&text) {
                let selection_active = loaded
                    .engine
                    .document()
                    .map(|d| d.selection().active())
                    .unwrap_or(false);
                if let Some(warning) = selection_scoped_warning(&kind, selection_active) {
                    eprintln!("{warning}");
                }
            }
            // WD-1: the raw text reaches the engine unmodified.
            let receipt = loaded
                .engine
                .apply_json(&text)
                .map_err(|e| Failure::from_engine(e).with_id(sniff_id(&text)))?;
            save_doc(&loaded.engine, doc)?;
            emit(
                cli,
                json!({ "ok": true, "receipt": receipt_json(&receipt) }),
                vec![cat.tr(
                    "cli.apply.ok",
                    &[
                        ("kind", receipt.kind.clone()),
                        ("id", receipt.id.to_string()),
                        ("revision", receipt.revision.to_string()),
                    ],
                )],
            );
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Script {
            doc,
            file,
            autosave,
        } => {
            let text = std::fs::read_to_string(file)
                .map_err(|e| Failure::io_read(&format!("reading {}", file.display()), e))?;
            let mut loaded = load_doc(doc)?;
            tune_engine(cli, &mut loaded.engine);
            // §7.8 autosave: BASE anchors to the package as loaded; each
            // applied envelope appends + checkpoints (headless per-command
            // durability policy — the interval cadence is the interactive
            // shell's). A hard kill mid-script then recovers to the last
            // applied command via `recover`.
            let mut log = match autosave {
                Some(path) => {
                    let doc_id = loaded
                        .engine
                        .document()
                        .map(|d| d.id().as_uuid())
                        .unwrap_or_else(Uuid::nil);
                    Some(
                        appname_core::io::autosave::Autosave::create(path, doc_id).map_err(
                            |e| {
                                Failure::io_read(
                                    &format!("creating autosave {}", path.display()),
                                    e,
                                )
                            },
                        )?,
                    )
                }
                None => None,
            };
            // Deterministic fault injection (W5/W22 kill-matrix style): a
            // TEST-ONLY door aborting the process (no destructors, no save)
            // after N applies — exercised by the CLI crash-recovery test.
            let fi_abort: Option<usize> = std::env::var("APPNAME_FI_ABORT_AFTER_APPLIES")
                .ok()
                .and_then(|v| v.parse().ok());
            let mut receipts = Vec::new();
            let mut applied = 0usize;
            for (i, raw) in text.lines().enumerate() {
                let line_no = i + 1;
                let raw = raw.trim();
                if raw.is_empty() || raw.starts_with('#') {
                    continue;
                }
                // Transactional: any failure aborts before save — the
                // package on disk stays byte-identical (P-D07).
                let receipt = loaded.engine.apply_json(raw).map_err(|e| {
                    let mut f = Failure::from_engine(e).with_id(sniff_id(raw));
                    f.line = Some(line_no);
                    f
                })?;
                if let Some(log) = log.as_mut() {
                    // Fix-round M5: append the envelope JUST APPLIED — the
                    // †-position form of history.goto journals as the
                    // equivalent undo/redo run (cursor movement, nothing
                    // appended), so `journal().last()` would duplicate the
                    // PREVIOUS envelope and the goto itself would be lost
                    // to §10.6 crash replay. Re-parsing the applied line
                    // yields exactly the applied envelope for every kind.
                    let env = appname_core::engine::envelope::Envelope::from_json(raw)
                        .map_err(Failure::from_engine)?;
                    log.append_command(&env)
                        .and_then(|_| log.checkpoint(Some(env.id)))
                        .map_err(|e| Failure::io_read("appending to the autosave journal", e))?;
                }
                applied += 1;
                if fi_abort == Some(applied) {
                    // Hard kill: the journal is durable up to here; the
                    // package on disk is untouched.
                    std::process::abort();
                }
                receipts.push(receipt_json(&receipt));
            }
            save_doc(&loaded.engine, doc)?;
            if let Some(log) = log.as_mut() {
                // §7.8 fold rule: a successful save IS the fold — the log
                // truncates to a fresh durable BASE.
                log.rebase()
                    .map_err(|e| Failure::io_read("rebasing the autosave journal", e))?;
            }
            emit(
                cli,
                json!({ "ok": true, "applied": receipts.len(), "receipts": receipts }),
                vec![cat.tr(
                    "cli.script.ok",
                    &[
                        ("applied", receipts.len().to_string()),
                        ("doc", doc.display().to_string()),
                    ],
                )],
            );
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Duplicate { doc, out } => {
            let loaded = load_doc(doc)?;
            let dup = loaded.engine.duplicate().map_err(Failure::from_engine)?;
            save_doc(&dup, out)?;
            emit(
                cli,
                json!({ "ok": true, "out": out.display().to_string(),
                        "journalLength": dup.journal().len(),
                        "note": "doc.duplicate is a dagger kind: no journal record on the source; the duplicate opens with empty history (§10.3/§10.6)" }),
                vec![cat.tr(
                    "cli.duplicate.ok",
                    &[
                        ("doc", doc.display().to_string()),
                        ("out", out.display().to_string()),
                    ],
                )],
            );
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Recover { doc, log, out } => {
            let (engine, parsed, replayed) =
                appname_core::io::autosave::recover(&full_assembly(), doc, log)
                    .map_err(Failure::from_engine)?;
            save_doc(&engine, out)?;
            emit(
                cli,
                json!({ "ok": true, "replayed": replayed, "torn": parsed.torn,
                        "warnings": parsed.warnings, "out": out.display().to_string(),
                        "journalLength": engine.journal().len() }),
                vec![cat.tr(
                    "cli.recover.ok",
                    &[
                        ("replayed", replayed.to_string()),
                        ("out", out.display().to_string()),
                        ("doc", doc.display().to_string()),
                    ],
                )],
            );
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Undo { doc, steps } | Cmd::Redo { doc, steps } => {
            let redo = matches!(cli.cmd, Cmd::Redo { .. });
            let loaded = load_doc(doc)?;
            let (engine, performed) =
                undo_redo(&loaded.engine, *steps as usize, redo).map_err(Failure::from_engine)?;
            save_doc(&engine, doc)?;
            let key = if redo { "cli.redo.ok" } else { "cli.undo.ok" };
            emit(
                cli,
                json!({ "ok": true, "performed": performed, "cursor": engine.journal_cursor(),
                        "journalLength": engine.journal().len(),
                        "entries": engine.journal_rows() }),
                vec![cat.tr(
                    key,
                    &[
                        ("performed", performed.to_string()),
                        ("cursor", engine.journal_cursor().to_string()),
                        ("length", engine.journal().len().to_string()),
                    ],
                )],
            );
            Ok(ExitCode::SUCCESS)
        }

        Cmd::History { doc } => {
            let loaded = load_doc(doc)?;
            let entries = loaded.engine.journal_rows();
            let mut human = vec![cat.tr(
                "cli.history.header",
                &[
                    ("length", entries.len().to_string()),
                    ("cursor", loaded.engine.journal_cursor().to_string()),
                ],
            )];
            for e in &entries {
                human.push(format!(
                    "  {:>4}  {}  {:<18}  {}{}",
                    e.index,
                    if e.active { "*" } else { " " },
                    e.kind,
                    e.id,
                    e.timestamp
                        .as_deref()
                        .map(|t| format!("  {t}"))
                        .unwrap_or_default(),
                ));
            }
            emit(
                cli,
                json!({ "ok": true, "length": entries.len(),
                        "cursor": loaded.engine.journal_cursor(), "entries": entries }),
                human,
            );
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Export {
            doc,
            format,
            out,
            quality,
        } => {
            if quality.is_some() && !(format == "jpeg" || format == "webp") {
                return Err(Failure::rejected(
                    "error.api.invalid_arg",
                    format!("--quality is only valid with --format jpeg|webp, not {format}"),
                ));
            }
            let loaded = load_doc(doc)?;
            let bytes = if format == "png" {
                export_bytes(&loaded.engine)? // byte-stable golden path
            } else {
                export_bytes_format(&loaded.engine, format, *quality)?
            };
            std::fs::write(out, &bytes).map_err(|e| {
                Failure::from_engine(EngineError::new(
                    ErrorCode::IoWriteFailed,
                    format!("writing {}: {e}", out.display()),
                ))
            })?;
            emit(
                cli,
                json!({ "ok": true, "out": out.display().to_string(), "format": format, "bytes": bytes.len() }),
                vec![cat.tr(
                    "cli.export.ok",
                    &[
                        ("out", out.display().to_string()),
                        ("format", format.clone()),
                        ("bytes", bytes.len().to_string()),
                    ],
                )],
            );
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Import {
            doc,
            image,
            as_layer,
        } => {
            // Ratified MAJOR-2 reconciliation: import = raster-asset
            // ingestion + a JOURNALED edit.paste envelope (§9.6 rule 2), so
            // replay reproduces imports byte-identically. `at` is submitted
            // explicitly as [0, 0] (the historical import placement).
            let mut loaded = load_doc(doc)?;
            tune_engine(cli, &mut loaded.engine);
            let bytes = std::fs::read(image)
                .map_err(|e| Failure::io_read(&format!("reading {}", image.display()), e))?;
            // Q-D5(a) IMPORT WIDENING, 2026-08-29: sniff through the codec
            // registry's allow-list and hand the engine the PNG carrier.
            // The extension is passed ONLY as the registry's documented
            // tie-break between equal-length magics -- the accept decision
            // is made on the RESOLVED codec id, so `evil.png` holding PSD
            // bytes is refused as psd rather than admitted as png.
            let carrier =
                appname_io::decode_for_ingress(bytes, image.extension().and_then(|e| e.to_str()))
                    .map_err(|e| {
                    Failure::from_engine(appname_core::EngineError::bad_target(e.to_string()))
                })?;
            let asset_id = loaded
                .engine
                .add_raster_asset(carrier)
                .map_err(Failure::from_engine)?;
            let new_layer = Uuid::new_v4();
            let envelope_id = Uuid::new_v4();
            let text = json!({
                "schema": "appname.command.v1",
                "id": envelope_id,
                "kind": "edit.paste",
                "params": { "assetId": asset_id, "at": [0, 0], "newLayerId": new_layer },
            })
            .to_string();
            loaded
                .engine
                .apply_json(&text)
                .map_err(|e| Failure::from_engine(e).with_id(Some(envelope_id.to_string())))?;
            save_doc(&loaded.engine, doc)?;
            let mut human = vec![cat.tr(
                "cli.import.ok",
                &[
                    ("layer", "Pasted".to_string()),
                    ("id", new_layer.to_string()),
                    ("doc", doc.display().to_string()),
                ],
            )];
            if as_layer.is_some() {
                human.push(
                    "note: --as-layer is inert since the journaled import (edit.paste names \
                     the layer \"Pasted\"; renaming arrives with layer.rename)"
                        .to_string(),
                );
            }
            emit(
                cli,
                json!({ "ok": true, "layerId": new_layer, "layerName": "Pasted",
                        "assetId": asset_id, "envelopeId": envelope_id,
                        "note": "journaled import: edit.paste over an ingested raster asset — replay reproduces it (MAJOR-2)",
                        "asLayerInert": as_layer.is_some() }),
                human,
            );
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Info { doc } => {
            let loaded = load_doc(doc)?;
            let engine = &loaded.engine;
            let d = engine.document().ok_or_else(|| {
                Failure::from_engine(EngineError::bad_target("package holds no document"))
            })?;
            let layers: Vec<Value> = d
                .z_order_walk()
                .iter()
                .enumerate()
                .filter_map(|(i, id)| {
                    d.layer(*id).map(|l| {
                        // q-tutorial/Q-7 (MAJOR): all five `op.*` kinds require
                        // `params.opId` and NO read door published one, so on any
                        // document you did not just author the adjustment is frozen
                        // — you cannot mask, retune, toggle, reorder or delete its
                        // op. The id exists (adjustment.add requires the caller to
                        // mint it); only the way back to it was missing. Additive,
                        // like the cabi door's own "info v2" block.
                        let ops: Vec<Value> = l
                            .adjustment_op()
                            .into_iter()
                            .chain(
                                l.smart_object()
                                    .into_iter()
                                    .flat_map(|so| so.smart_filters().iter().copied()),
                            )
                            .filter_map(|oid| {
                                d.op(oid).map(|o| {
                                    json!({ "id": oid.as_uuid(), "kind": o.op(), "enabled": o.enabled() })
                                })
                            })
                            .collect();
                        json!({
                            "index": i,
                            "id": l.id(),
                            "name": l.name(),
                            "visible": l.visible(),
                            "opacity": l.opacity(),
                            "kind": l.kind().kind_str(),
                            "ops": ops,
                        })
                    })
                })
                .collect();
            let cursor = engine.journal_cursor();
            let info = json!({
                "docId": d.id(),
                "width": d.geometry().canvas_size.x,
                "height": d.geometry().canvas_size.y,
                // ABI 1.7 §19.1 output frame — SAME KEY, SAME EXPRESSION as
                // the cabi info door emits. Two info emitters exist (this one
                // and appname-cabi's); publishing the same concept under one
                // spelling with one computation is what keeps them from
                // becoming a shared name with two meanings.
                "imageRect": [
                    d.geometry().image_rect().min.x,
                    d.geometry().image_rect().min.y,
                    d.geometry().image_rect().width(),
                    d.geometry().image_rect().height()
                ],
                "layers": layers,
                "selection": { "active": d.selection().active() },
                "journal": {
                    "length": engine.journal().len(),
                    "cursor": cursor,
                    "canUndo": cursor > 1,
                    "canRedo": cursor < engine.journal().len(),
                },
                "warnings": loaded.warnings.len(),
            });
            let mut human = vec![cat.tr(
                "cli.info.doc",
                &[
                    ("docId", d.id().to_string()),
                    ("width", d.geometry().canvas_size.x.to_string()),
                    ("height", d.geometry().canvas_size.y.to_string()),
                ],
            )];
            human.push(cat.tr("cli.info.layers", &[("count", layers.len().to_string())]));
            for l in &layers {
                human.push(format!(
                    "  [{}] {:<20} {}  visible={} opacity={:.3}",
                    l["index"],
                    l["name"].as_str().unwrap_or(""),
                    l["id"].as_str().unwrap_or(""),
                    l["visible"],
                    l["opacity"].as_f64().unwrap_or(1.0)
                ));
            }
            human.push(cat.tr(
                "cli.info.journal",
                &[
                    ("length", info["journal"]["length"].to_string()),
                    ("cursor", info["journal"]["cursor"].to_string()),
                    ("canUndo", info["journal"]["canUndo"].to_string()),
                    ("canRedo", info["journal"]["canRedo"].to_string()),
                ],
            ));
            emit(cli, json!({ "ok": true, "info": info }), human);
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Asset {
            cmd: AssetCmd::Add { doc, file },
        } => {
            let bytes = std::fs::read(file)
                .map_err(|e| Failure::io_read(&format!("reading {}", file.display()), e))?;
            // Type sanity before registering: the blob must parse as
            // appname.samples.v1 (opaque to core; decoded here at the
            // assembly point).
            appname_tools::brush::decode_samples(&bytes).map_err(|e| {
                Failure::rejected(
                    "error.command.bad_target",
                    format!("not an appname.samples.v1 blob: {e}"),
                )
            })?;
            let mut loaded = load_doc(doc)?;
            tune_engine(cli, &mut loaded.engine);
            // q-tutorial/Q-5 (MAJOR): every `asset add` is its own process, so
            // the assets registered by earlier invocations arrive unreferenced
            // and no longer pending, and this save would GC them — three adds
            // used to leave one. Re-arm the grace over what the package
            // already holds before adding this one.
            loaded.engine.rearm_pending_assets();
            let asset_id = loaded.engine.add_samples_asset(bytes.clone());
            save_doc(&loaded.engine, doc)?;
            emit(
                cli,
                json!({ "ok": true, "assetId": asset_id, "bytes": bytes.len(),
                        "note": "asset ids are content-derived (same bytes, same id); assets persist with the journal entries that reference them" }),
                vec![cat.tr(
                    "cli.asset.add.ok",
                    &[
                        ("assetId", asset_id.to_string()),
                        ("bytes", bytes.len().to_string()),
                        ("doc", doc.display().to_string()),
                    ],
                )],
            );
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Asset {
            cmd: AssetCmd::AddImage { doc, file },
        } => {
            let bytes = std::fs::read(file)
                .map_err(|e| Failure::io_read(&format!("reading {}", file.display()), e))?;
            let mut loaded = load_doc(doc)?;
            tune_engine(cli, &mut loaded.engine);
            // q-tutorial/Q-5's sibling door: same process boundary, same GC.
            loaded.engine.rearm_pending_assets();
            // Decode-validated at the engine door (§9.2): garbage refuses
            // here, not at every paste. Q-D5(a): widened past PNG through
            // the allow-listed ingress decoder first.
            let carrier = appname_io::decode_for_ingress(
                bytes.clone(),
                file.extension().and_then(|e| e.to_str()),
            )
            .map_err(|e| {
                Failure::from_engine(appname_core::EngineError::bad_target(e.to_string()))
            })?;
            let asset_id = loaded
                .engine
                .add_raster_asset(carrier)
                .map_err(Failure::from_engine)?;
            save_doc(&loaded.engine, doc)?;
            emit(
                cli,
                json!({ "ok": true, "assetId": asset_id, "bytes": bytes.len(), "kind": "rasterImage",
                        "note": "asset ids are content-derived (same bytes, same id); reference via edit.paste params.assetId" }),
                vec![cat.tr(
                    "cli.asset.add.ok",
                    &[
                        ("assetId", asset_id.to_string()),
                        ("bytes", bytes.len().to_string()),
                        ("doc", doc.display().to_string()),
                    ],
                )],
            );
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Selftest { keep } => {
            let report = selftest::run(*keep);
            if cli.json {
                let rows: Vec<Value> = report
                    .rows
                    .iter()
                    .map(|r| json!({ "name": r.name, "status": if r.pass { "PASS" } else { "FAIL" }, "detail": r.detail }))
                    .collect();
                println!(
                    "{}",
                    json!({ "ok": report.pass, "checks": rows, "workDir": report.work_dir })
                );
            } else {
                println!("{}", cat.tr("cli.selftest.header", &[]));
                for r in &report.rows {
                    println!(
                        "  {:<44} {:<4}  {}",
                        r.name,
                        if r.pass { "PASS" } else { "FAIL" },
                        r.detail
                    );
                }
                let passed = report.rows.iter().filter(|r| r.pass).count();
                let key = if report.pass {
                    "cli.selftest.pass"
                } else {
                    "cli.selftest.fail"
                };
                println!(
                    "{}",
                    cat.tr(
                        key,
                        &[
                            ("passed", passed.to_string()),
                            ("total", report.rows.len().to_string()),
                            ("golden", "self-contained".to_string()),
                        ],
                    )
                );
            }
            Ok(if report.pass {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_scoped_warning_fires_only_without_a_selection() {
        // Selection-consuming kind, NO active selection → warn (the P-6
        // silently-destructive case).
        let w = selection_scoped_warning("fill", false).expect("fill with no selection warns");
        assert!(w.contains("no active selection"), "{w}");
        assert!(w.contains("WHOLE layer"), "{w}");
        // Same kind WITH an active selection → no warning.
        assert!(selection_scoped_warning("fill", true).is_none());
        // A kind that does not scope to the selection → never warns.
        assert!(selection_scoped_warning("layer.add", false).is_none());
        assert!(selection_scoped_warning("doc.crop", false).is_none());
        // A kind that HARD-FAILS on no selection is excluded (it refuses
        // loudly on its own — not the silent-destructive class).
        assert!(selection_scoped_warning("content_aware_fill", false).is_none());
        assert!(selection_scoped_warning("edit.paste_into", false).is_none());
    }

    #[test]
    fn selection_scoped_set_is_sorted_and_unique() {
        let mut sorted = SELECTION_SCOPED_KINDS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.as_slice(),
            SELECTION_SCOPED_KINDS,
            "SELECTION_SCOPED_KINDS must stay sorted + unique"
        );
    }

    /// The guard set is DERIVED from the frozen contracts' `selectionBehavior`
    /// field: a kind is selection-consuming-with-silent-fallback iff its
    /// contract "gates coverage" (plus `edit.cut`, whose "no selection =
    /// layer bounds (whole content clears)" is the same silent-whole-content
    /// hazard). This re-derives that set from `contracts/commands/` and fails
    /// if `SELECTION_SCOPED_KINDS` drifts from it — so a bogos-side contract
    /// change (a new gates-coverage kind, or a re-classification) is caught
    /// here rather than silently un-guarded.
    #[test]
    fn selection_scoped_set_matches_contracts() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/commands");
        assert!(
            dir.is_dir(),
            "contract mirror not found at {} — the cross-check cannot run",
            dir.display()
        );

        let mut derived: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&dir).expect("read contracts/commands") {
            let path = entry.expect("dir entry").path();
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            if path.extension().and_then(|e| e.to_str()) != Some("yaml") || stem == "_index" {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read contract");
            if selection_behavior_gates_coverage(&text) || stem == "edit.cut" {
                derived.push(stem);
            }
        }
        derived.sort();
        derived.dedup();

        let expect: Vec<String> = SELECTION_SCOPED_KINDS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            derived, expect,
            "SELECTION_SCOPED_KINDS drifted from the contracts' selectionBehavior field.\n  \
             from contracts: {derived:?}\n  in code:        {expect:?}"
        );
    }

    /// Extract the `selectionBehavior:` value/block from a contract's YAML
    /// text (the value line plus any indented block-scalar continuation, up
    /// to the next top-level key) and report whether it declares "gates
    /// coverage".
    fn selection_behavior_gates_coverage(contract_yaml: &str) -> bool {
        let mut in_block = false;
        let mut captured = String::new();
        for line in contract_yaml.lines() {
            if in_block {
                // Block continues while the line is blank or indented.
                if line.is_empty() || line.starts_with(char::is_whitespace) {
                    captured.push(' ');
                    captured.push_str(line);
                    continue;
                }
                break; // next top-level key ends the block
            }
            if let Some(rest) = line.strip_prefix("selectionBehavior:") {
                captured.push_str(rest);
                in_block = true;
            }
        }
        captured.contains("gates coverage")
    }
}
