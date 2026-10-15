// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §26.2 batch processing: one action over N input files, with the
//! [NORMATIVE] output-template grammar, per-file step reports, and
//! `batch-report.json` (`appname.batchreport.v1`).
//!
//! Spec: ES §26.2 (batch job/report, on-error policy, collision policy),
//! §26.1 (replay semantics — delegated to [`crate::replay`]), W20
//! groundwork (plan §2.4 row "Actions/macros v1"; the CLI `--batch` mode
//! wiring is O-CORE's `appname-cli` and consumes this module).
//!
//! # Inputs and outputs at this checkpoint
//!
//! Inputs: `.appname` packages (loaded over the caller's assembly) and
//! PNG files (ingested exactly like the CLI's journaled import: raster
//! asset + `edit.paste` into a fresh document — MAJOR-2 shape, so the
//! per-file journal replays byte-identically). Outputs go through the
//! [`Exporter`] seam; the built-in [`PngExporter`] uses the ONE
//! compositing truth (render's parity-gated `composite_document_with`
//! OVER THE ENGINE'S §12.1 OP REGISTRY — `engine.assembly().ops()`, so
//! adjustment/smart-chain documents serve exactly as through cli/cabi
//! [O-INT MAJOR-2, consolidated fix round: this arm carried a bare
//! registry and refused them] — + core's §6.4 PNG encoder,
//! single-raster fast path on core's reference bytes — the same two
//! arms as the ABI export door). Other formats ride
//! the export door of whichever front end drives the batch (the CLI's
//! dispatch or the C ABI's `appname_doc_export`) by implementing
//! [`Exporter`]; growing a second engine-side codec assembly here would
//! fork export truth (lane decision, log-noted).

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use uuid::Uuid;

use appname_core::engine::kind_impl::EngineAssembly;
use appname_core::io::package::load_package_with;
use appname_core::{Engine, EngineError, ErrorCode};

use crate::action::Action;
use crate::replay::{replay, EngineDoor, ReplayOptions, ReplayOutcome, ReplayReport};
use crate::template::{resolve_collision, substitute, TemplateVars};

/// Civil UTC timestamp `(year, month, day, hour, minute, second)`.
pub type CivilUtc = (i64, u32, u32, u32, u32, u32);

/// Wall clock as civil UTC — the default [`BatchJob::clock`] and the
/// front ends' `started` source. Howard Hinnant's civil-from-days
/// (public-domain algorithm form).
pub fn now_civil_utc() -> CivilUtc {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        m,
        d,
        (sod / 3600) as u32,
        (sod % 3600 / 60) as u32,
        (sod % 60) as u32,
    )
}

/// §26.2 on-error policy (CLI `--on-error`, default continue).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnError {
    #[default]
    Continue,
    Halt,
}

/// Output writer seam (module docs).
pub trait Exporter {
    /// The §26.2 `{ext}` value for `format` (canonical, lowercase, no
    /// dot), or `None` when the format is unknown to this exporter.
    fn canonical_ext(&self, format: &str) -> Option<String>;
    /// Encode the flattened document to `path`.
    fn export(&mut self, engine: &Engine, path: &Path, format: &str) -> Result<(), EngineError>;
}

/// Built-in PNG exporter: the one-compositing-truth assembly (module
/// docs). PNG only; other format ids answer typed
/// `error.command.not_implemented` through [`Exporter::canonical_ext`]
/// returning `None`.
#[derive(Debug, Default)]
pub struct PngExporter;

impl Exporter for PngExporter {
    fn canonical_ext(&self, format: &str) -> Option<String> {
        (format == "png").then(|| "png".to_string())
    }

    fn export(&mut self, engine: &Engine, path: &Path, format: &str) -> Result<(), EngineError> {
        if format != "png" {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                format!(
                    "batch export format {format:?} is PNG-only at this checkpoint \
                     (multi-format batches ride a front end's export door)"
                ),
            ));
        }
        let doc = engine
            .document()
            .ok_or_else(|| EngineError::bad_target("document not initialized after replay"))?;
        // NON-PARAMETRIC single-raster fast path: core's §4.4 reference
        // bytes (the goldens bar) — the same arm the ABI export door and
        // the CLI dispatch take, gated IDENTICALLY. The
        // `!has_visible_parametric_content` guard is load-bearing: without
        // it a ≤1-raster doc carrying a parametric shape/text layer with
        // NON-DEFAULT blend/opacity/fill_opacity would route to core's
        // §4.4-source-over-only `export_png_bytes`, which REFUSES typed
        // (png_io parametric_sources), while cli/cabi route it to render and
        // SERVE — the exact serve↔refuse split Option A collapses. Parametric
        // content (adjustment, smart, AND shape/text) routes through render
        // below at ANY raster count; only pure-raster single-raster docs keep
        // the fast path (byte-identical to render, pinned).
        let rasters = doc
            .z_order_walk()
            .iter()
            .filter_map(|id| doc.layer(*id))
            .filter(|n| n.pixels().is_some())
            .count();
        let bytes = if !appname_core::has_visible_parametric_content(doc) && rasters <= 1 {
            appname_core::io::png_io::export_png_bytes(engine)?
        } else {
            // The general arm: the parity-gated compositor over the
            // ENGINE'S FULL assembly (§12.1 ops + the Door-6/Door-1/fill-math
            // seams) — the same call cli/cabi make, and now gated identically
            // (O-INT MAJOR-1 fix: the missing `!has_visible_parametric_content`
            // guard retained a serve↔refuse split at this 4th door; O-INT
            // MAJOR-2 earlier fixed the bare-registry adjustment refusal).
            // [Cross-fence forced touch: the composite-legs slice widened
            // `composite_document_with`'s seam argument from `&OpAssembly` to
            // the seam-carrying `&EngineAssembly`; flagged for O-SCRIPT
            // ratification.]
            // §19.1 one compositing truth: frame on `image_rect()` (the crop
            // sub-region while a crop is active, else the full canvas) — the
            // same frame core's fast path, cli, and cabi honor.
            let frame = doc.geometry().image_rect();
            let composite = appname_render::composite_document_with(doc, engine.assembly(), frame)
                .map_err(|e| {
                    EngineError::new(
                        ErrorCode::NotImplemented,
                        format!("render composite refused: {e}"),
                    )
                })?;
            let plane = composite.region_rgba_f32(frame);
            appname_core::io::png_io::encode_composite_png(
                frame.width() as u32,
                frame.height() as u32,
                &plane,
            )?
        };
        std::fs::write(path, bytes).map_err(|e| {
            EngineError::new(
                ErrorCode::IoWriteFailed,
                format!("writing {}: {e}", path.display()),
            )
        })
    }
}

/// One batch job (§26.2 `BatchJob`).
pub struct BatchJob<'a> {
    pub action: &'a Action,
    pub inputs: Vec<PathBuf>,
    /// §26.2 output template; relative templates resolve against the
    /// current directory. Literal directory components are created.
    pub output_template: String,
    /// Output codec id fed to the exporter (`"png"` for the built-in).
    pub format: String,
    pub on_error: OnError,
    /// Disables the ` (n)` collision suffix.
    pub overwrite: bool,
    /// Replay-time variables + active layer (per file, identical).
    pub replay: ReplayOptions,
    /// Job start civil UTC time for `{date:…}` — the caller supplies it
    /// (the CLI passes wall clock; tests pass a constant; the report
    /// echoes it in RFC 3339 form).
    pub started: CivilUtc,
    /// Clock read once when the batch finishes, feeding the report's
    /// `finished` field with the REAL finish time (never a restatement
    /// of `started`). `None` = wall clock ([`now_civil_utc`]); a test
    /// that needs deterministic report bytes injects a constant.
    pub clock: Option<fn() -> CivilUtc>,
}

/// One per-file result row.
#[derive(Debug)]
pub struct FileResult {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub replay: Option<ReplayReport>,
    /// Non-step failure (open/export), or a summary of the step failure.
    pub error: Option<EngineError>,
}

impl FileResult {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }

    /// True when this row failed AT A STEP of the action (§26.1 replay
    /// stopped on `ReplayOutcome::Failed`) — the only rows that carry
    /// the `error.batch.step_failed` messageId. Open/export/template
    /// failures answer `false`.
    pub fn step_failure(&self) -> bool {
        matches!(
            &self.replay,
            Some(rep) if matches!(rep.outcome, ReplayOutcome::Failed { .. })
        )
    }
}

/// The whole-batch report (§26.2 `BatchReport`).
#[derive(Debug)]
pub struct BatchReport {
    pub action_name: String,
    pub action_id: Option<Uuid>,
    pub started_rfc3339: String,
    /// Real finish time ([`BatchJob::clock`] read after the last row).
    pub finished_rfc3339: String,
    pub results: Vec<FileResult>,
    /// Files not attempted because `OnError::Halt` stopped the batch.
    pub skipped: usize,
}

impl BatchReport {
    pub fn all_ok(&self) -> bool {
        self.skipped == 0 && self.results.iter().all(FileResult::ok)
    }

    /// The `appname.batchreport.v1` wire document (§26.2 [NORMATIVE]).
    pub fn to_value(&self) -> Value {
        let results: Vec<Value> = self
            .results
            .iter()
            .map(|r| {
                let mut row = Map::new();
                row.insert("input".into(), json!(r.input.display().to_string()));
                row.insert(
                    "output".into(),
                    r.output
                        .as_ref()
                        .map(|p| json!(p.display().to_string()))
                        .unwrap_or(Value::Null),
                );
                row.insert("status".into(), json!(if r.ok() { "ok" } else { "error" }));
                if let Some(rep) = &r.replay {
                    row.insert(
                        "steps".into(),
                        rep.to_value().get("steps").cloned().unwrap_or_default(),
                    );
                }
                if let Some(e) = &r.error {
                    // `error.batch.step_failed` names STEP failures only.
                    // Non-step failures (open/export/unsupported input)
                    // are detail-only rows: typed code + detail, no
                    // messageId — the proper second id is an O-I18N
                    // fold-in candidate, not minted here (O-INT interim
                    // ruling, lane log fix-round entry).
                    let mut error = Map::new();
                    error.insert("code".into(), json!(e.code.as_str()));
                    if r.step_failure() {
                        error.insert("messageId".into(), json!("error.batch.step_failed"));
                    }
                    error.insert("detail".into(), json!(e.message));
                    row.insert("error".into(), Value::Object(error));
                }
                Value::Object(row)
            })
            .collect();
        let ok = self.results.iter().filter(|r| r.ok()).count();
        let failed = self.results.len() - ok;
        json!({
            "schema": "appname.batchreport.v1",
            "action": self.action_name,
            "actionId": self.action_id.map(|u| u.to_string()),
            "started": self.started_rfc3339,
            "finished": self.finished_rfc3339,
            "results": results,
            "summary": {
                "total": self.results.len() + self.skipped,
                "ok": ok,
                "failed": failed,
                "skipped": self.skipped,
            },
        })
    }
}

fn rfc3339(t: CivilUtc) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        t.0, t.1, t.2, t.3, t.4, t.5
    )
}

/// Run the batch. `assembly` is the kind assembly documents open over
/// (front ends pass their full registered set). The report is returned
/// AND written as `batch-report.json` next to the outputs (§26.2); a
/// report-write failure is the job-level `Err` (the per-file work is in
/// the returned value's history regardless — callers decide how loudly
/// to fail).
pub fn run_batch(
    job: &BatchJob<'_>,
    assembly: &EngineAssembly,
    exporter: &mut dyn Exporter,
) -> Result<BatchReport, EngineError> {
    // Template + format validation up front (§26.2: validation errors are
    // job-level, CLI exit 2 — not per-file rows).
    let ext = exporter.canonical_ext(&job.format).ok_or_else(|| {
        EngineError::new(
            ErrorCode::NotImplemented,
            format!("unknown batch output format {:?}", job.format),
        )
    })?;
    {
        // Dry-run the template so unknown variables reject before any
        // file is touched.
        let probe = TemplateVars {
            name: "probe",
            ext: &ext,
            seq: 1,
            w: 0,
            h: 0,
            started: job.started,
        };
        substitute(&job.output_template, &probe)?;
    }

    let mut report = BatchReport {
        action_name: job.action.name.clone(),
        action_id: job.action.id,
        started_rfc3339: rfc3339(job.started),
        finished_rfc3339: String::new(), // set below, after the last row
        results: Vec::with_capacity(job.inputs.len()),
        skipped: 0,
    };
    let mut report_dir: Option<PathBuf> = None;

    for (i, input) in job.inputs.iter().enumerate() {
        let seq = i + 1;
        let outcome = run_one(job, assembly, exporter, input, seq, &ext);
        let halt = !outcome.ok() && job.on_error == OnError::Halt;
        if let Some(out) = &outcome.output {
            if report_dir.is_none() {
                report_dir = out.parent().map(Path::to_path_buf);
            }
        }
        report.results.push(outcome);
        if halt {
            report.skipped = job.inputs.len() - seq;
            break;
        }
    }

    // The REAL finish time (MINOR-4 ruling: the shipped field must not
    // misstate) — read after the last row, before the report writes.
    report.finished_rfc3339 = rfc3339(job.clock.map_or_else(now_civil_utc, |c| c()));

    // §26.2: written next to the outputs (fall back to the template's
    // literal directory, then the current directory).
    let dir = report_dir.unwrap_or_else(|| {
        Path::new(&job.output_template)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    });
    std::fs::create_dir_all(&dir).ok();
    let report_path = dir.join("batch-report.json");
    std::fs::write(&report_path, report.to_value().to_string()).map_err(|e| {
        EngineError::new(
            ErrorCode::IoWriteFailed,
            format!("writing {}: {e}", report_path.display()),
        )
    })?;
    Ok(report)
}

fn failure(input: &Path, error: EngineError) -> FileResult {
    FileResult {
        input: input.to_path_buf(),
        output: None,
        replay: None,
        error: Some(error),
    }
}

fn run_one(
    job: &BatchJob<'_>,
    assembly: &EngineAssembly,
    exporter: &mut dyn Exporter,
    input: &Path,
    seq: usize,
    ext: &str,
) -> FileResult {
    // 1. Open.
    let mut engine = match open_input(assembly, input) {
        Ok(e) => e,
        Err(e) => return failure(input, e),
    };

    // 2. Replay through the baseline engine door.
    let rep = {
        let mut door = EngineDoor(&mut engine);
        replay(job.action, &mut door, &job.replay)
    };
    if let ReplayOutcome::Failed { at, error } = &rep.outcome {
        let summary = EngineError::new(
            error.code,
            format!(
                "step {at} ({}) failed: {}",
                step_kind(job.action, *at),
                error.message
            ),
        );
        return FileResult {
            input: input.to_path_buf(),
            output: None,
            replay: Some(rep),
            error: Some(summary),
        };
    }

    // 3. Substitute the output path ({w}/{h} are POST-processing dims).
    let (w, h) = match engine.document() {
        Some(d) => (
            d.geometry().canvas_size.x as u32,
            d.geometry().canvas_size.y as u32,
        ),
        None => (0, 0),
    };
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let vars = TemplateVars {
        name: &stem,
        ext,
        seq,
        w,
        h,
        started: job.started,
    };
    let resolved = match substitute(&job.output_template, &vars) {
        Ok(p) => PathBuf::from(p),
        Err(e) => return failure(input, e),
    };
    if let Some(parent) = resolved.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return failure(
                    input,
                    EngineError::new(
                        ErrorCode::IoWriteFailed,
                        format!("creating {}: {e}", parent.display()),
                    ),
                );
            }
        }
    }
    let out_path = resolve_collision(&resolved, job.overwrite);

    // 4. Export.
    if let Err(e) = exporter.export(&engine, &out_path, &job.format) {
        return FileResult {
            input: input.to_path_buf(),
            output: None,
            replay: Some(rep),
            error: Some(e),
        };
    }
    FileResult {
        input: input.to_path_buf(),
        output: Some(out_path),
        replay: Some(rep),
        error: None,
    }
}

fn step_kind(action: &Action, index: usize) -> String {
    match action.steps.get(index) {
        Some(crate::action::Step::Command(c)) => c
            .command
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string(),
        _ => "?".to_string(),
    }
}

/// Open one batch input: `.appname` packages load over the assembly; PNG
/// ingests as the CLI's journaled import (raster asset + `edit.paste`
/// into a fresh canvas-sized document — MAJOR-2 shape).
fn open_input(assembly: &EngineAssembly, path: &Path) -> Result<Engine, EngineError> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "appname" => Ok(load_package_with(assembly, path)?.engine),
        "png" => {
            let bytes = std::fs::read(path).map_err(|e| {
                EngineError::new(
                    ErrorCode::IoReadFailed,
                    format!("reading {}: {e}", path.display()),
                )
            })?;
            let (_, w, h) = appname_core::io::png_io::decode_png_to_buffer(
                &bytes,
                appname_core::graph::id::ProfileId::nil(),
            )?;
            let mut engine = assembly.build();
            engine.apply_json(
                &json!({
                    "schema": "appname.command.v1",
                    "id": Uuid::new_v4().to_string(),
                    "kind": "doc.new",
                    "params": { "width": w, "height": h, "background": "transparent" },
                })
                .to_string(),
            )?;
            let asset_id = engine.add_raster_asset(bytes)?;
            engine.apply_json(
                &json!({
                    "schema": "appname.command.v1",
                    "id": Uuid::new_v4().to_string(),
                    "kind": "edit.paste",
                    "params": { "assetId": asset_id, "at": [0, 0],
                                "newLayerId": Uuid::new_v4().to_string() },
                })
                .to_string(),
            )?;
            Ok(engine)
        }
        other => Err(EngineError::new(
            ErrorCode::InvalidArg,
            format!(
                "unsupported batch input {} (extension {other:?}; .appname and .png at \
                 this checkpoint)",
                path.display()
            ),
        )),
    }
}
