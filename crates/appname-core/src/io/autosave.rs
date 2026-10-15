// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §7.8 (autosave journal — record framing, fsync rules, torn-tail
//! truncation, fold-into-package), §10.6 (crash recovery: replay = base
//! state + commands), §40.1 quartet row `replay_cmd_journal[kind]`; W5
//! (SIGKILL mid-edit, recover to last journaled step).
//!
//! The STANDALONE autosave log — the on-scratch-disk sibling of the
//! in-package `journal/autosave.log` entry (whose §7.8 framing the package
//! module already implements). One log per document, append-only:
//!
//! - `BASE` (0x00) anchors replay to a saved `.appname` package;
//! - `COMMAND` (0x01) carries one `appname.command.v1` envelope;
//! - `CHECKPOINT` (0x03) is the durability point — the file is fsync'd;
//! - `ASSET` (0x04, reserved-SKIPPABLE range) carries a session-registered
//!   asset blob (uuid + kind tag + bytes). §7.8 defines 0x04–0x7F as
//!   skip-by-len for old readers, which is exactly what an asset frame
//!   needs; the frame is a divergence-noted pre-spec extension (paste
//!   envelopes reference assets, and D-48 replay needs the captured
//!   values). TILEREF (0x02) frames arrive with the after-image
//!   optimization; command-only replay is complete without them because
//!   every mutation is an envelope (D-48 — core has no out-of-journal
//!   mutations since the MAJOR-2 reconciliation).
//!
//! Fsync policy: [`Autosave::checkpoint`] fsyncs; the §7.8 cadence rules
//! (interval / CheckpointOnly / idle) belong to the interactive shell —
//! headless consumers (CLI `--autosave`) checkpoint per command, trading
//! §7.8's "< 0.5 ms enqueue" budget for the strongest crash floor
//! (divergence noted). Timestamps use the deterministic epoch placeholder
//! until the wall-clock cadence work lands (divergence noted).
//!
//! Torn-tail rule [NORMATIVE §7.8]: scanning stops at the first record
//! whose length exceeds the remaining bytes, whose CRC mismatches, or that
//! carries a reserved-critical type — the tail is truncated, never fatal.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::engine::assets::AssetKind;
use crate::engine::envelope::Envelope;
use crate::engine::error::{EngineError, ErrorCode};
use crate::engine::kind_impl::EngineAssembly;
use crate::engine::Engine;
use crate::io::integrity;
use crate::io::package::{record, EPOCH_TS};

const FRAME_BASE: u8 = 0x00;
const FRAME_COMMAND: u8 = 0x01;
const FRAME_CHECKPOINT: u8 = 0x03;
/// Reserved-skippable range (§7.8): asset carriage.
const FRAME_ASSET: u8 = 0x04;

/// Append-side handle for one document's autosave log.
pub struct Autosave {
    path: PathBuf,
    file: File,
    doc_id: Uuid,
}

impl Autosave {
    /// Create (truncate) the log and write the BASE anchor durably.
    pub fn create(path: &Path, doc_id: Uuid) -> std::io::Result<Autosave> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        let mut a = Autosave {
            path: path.to_path_buf(),
            file,
            doc_id,
        };
        a.append_base()?;
        a.file.sync_all()?;
        Ok(a)
    }

    fn append_base(&mut self) -> std::io::Result<()> {
        let mut body = Vec::with_capacity(16 + EPOCH_TS.len());
        body.extend_from_slice(self.doc_id.as_bytes());
        body.extend_from_slice(EPOCH_TS.as_bytes());
        self.append_record(FRAME_BASE, &body)
    }

    fn append_record(&mut self, rtype: u8, body: &[u8]) -> std::io::Result<()> {
        let mut frame = Vec::with_capacity(9 + body.len());
        record(&mut frame, rtype, body);
        self.file.write_all(&frame)
    }

    /// Append one applied envelope (buffered; durable at the next
    /// [`Autosave::checkpoint`]).
    pub fn append_command(&mut self, env: &Envelope) -> std::io::Result<()> {
        self.append_record(FRAME_COMMAND, env.to_json().as_bytes())
    }

    /// Append a session-registered asset blob (captured value the journal's
    /// envelopes reference — §9.6 rule 2 / D-48).
    pub fn append_asset(&mut self, id: Uuid, kind: AssetKind, bytes: &[u8]) -> std::io::Result<()> {
        let mut body = Vec::with_capacity(17 + bytes.len());
        body.extend_from_slice(id.as_bytes());
        body.push(match kind {
            AssetKind::Samples => 0,
            AssetKind::RasterImage => 1,
            AssetKind::FontFace => 2,
            AssetKind::MaskPlane => 3,
        });
        body.extend_from_slice(bytes);
        self.append_record(FRAME_ASSET, &body)
    }

    /// §7.8 durability point: CHECKPOINT frame + fsync.
    pub fn checkpoint(&mut self, last_command: Option<Uuid>) -> std::io::Result<()> {
        let mut body = Vec::with_capacity(EPOCH_TS.len() + 16);
        body.extend_from_slice(EPOCH_TS.as_bytes());
        body.extend_from_slice(last_command.unwrap_or_else(Uuid::nil).as_bytes());
        self.append_record(FRAME_CHECKPOINT, &body)?;
        self.file.sync_all()
    }

    /// §7.8 fold-into-package: a successful save IS the fold — truncate the
    /// log to a fresh durable BASE (the package now carries everything).
    pub fn rebase(&mut self) -> std::io::Result<()> {
        self.file.set_len(0)?;
        use std::io::Seek;
        self.file.seek(std::io::SeekFrom::Start(0))?;
        self.append_base()?;
        self.file.sync_all()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// One parsed log: everything recoverable before the (possibly torn) tail.
pub struct RecoveredLog {
    pub doc_id: Uuid,
    pub commands: Vec<Envelope>,
    pub assets: Vec<(Uuid, AssetKind, Vec<u8>)>,
    /// True when scanning stopped early (torn/corrupt tail — §7.8: costs
    /// the tail, never the document).
    pub torn: bool,
    pub warnings: Vec<String>,
}

/// Scan a log with the §7.8 truncation semantics.
pub fn read_log(path: &Path) -> std::io::Result<RecoveredLog> {
    let bytes = std::fs::read(path)?;
    let mut out = RecoveredLog {
        doc_id: Uuid::nil(),
        commands: Vec::new(),
        assets: Vec::new(),
        torn: false,
        warnings: Vec::new(),
    };
    let mut pos = 0usize;
    loop {
        if pos == bytes.len() {
            break;
        }
        if bytes.len() - pos < 8 {
            out.torn = true;
            out.warnings
                .push("torn frame header at tail (truncated)".into());
            break;
        }
        let len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap());
        if len == 0 || bytes.len() - pos - 8 < len {
            out.torn = true;
            out.warnings
                .push("frame length exceeds remaining bytes (truncated tail)".into());
            break;
        }
        let payload = &bytes[pos + 8..pos + 8 + len];
        if integrity::crc32(payload) != crc {
            out.torn = true;
            out.warnings
                .push("frame CRC mismatch (truncated at corrupt record)".into());
            break;
        }
        pos += 8 + len;
        match payload[0] {
            FRAME_BASE => {
                if payload.len() < 17 {
                    out.torn = true;
                    out.warnings.push("short BASE record (truncated)".into());
                    break;
                }
                out.doc_id = Uuid::from_bytes(payload[1..17].try_into().unwrap());
            }
            FRAME_COMMAND => {
                match std::str::from_utf8(&payload[1..])
                    .ok()
                    .and_then(|t| Envelope::from_json(t).ok())
                {
                    Some(env) => out.commands.push(env),
                    None => {
                        out.torn = true;
                        out.warnings
                            .push("unparseable COMMAND record (truncated here)".into());
                        break;
                    }
                }
            }
            FRAME_ASSET => {
                if payload.len() < 18 {
                    out.warnings.push("short ASSET record (skipped)".into());
                    continue;
                }
                let id = Uuid::from_bytes(payload[1..17].try_into().unwrap());
                let kind = match payload[17] {
                    1 => AssetKind::RasterImage,
                    2 => AssetKind::FontFace,
                    3 => AssetKind::MaskPlane,
                    _ => AssetKind::Samples,
                };
                out.assets.push((id, kind, payload[18..].to_vec()));
            }
            FRAME_CHECKPOINT => {}
            t if (0x04..=0x7F).contains(&t) => {
                // Reserved-skippable: skip by len (§7.8).
            }
            _ => {
                out.torn = true;
                out.warnings
                    .push("reserved-critical record type (replay stops)".into());
                break;
            }
        }
    }
    Ok(out)
}

/// §10.6 recovery: load the base package through `assembly`, register the
/// log's asset frames, then re-apply every journaled command not already in
/// the base's journal (idempotent over a base that raced ahead of the log).
/// Returns the recovered engine + how many commands replayed.
pub fn recover(
    assembly: &EngineAssembly,
    base_package: &Path,
    log_path: &Path,
) -> Result<(Engine, RecoveredLog, usize), EngineError> {
    let log = read_log(log_path).map_err(|e| {
        EngineError::new(
            ErrorCode::IoReadFailed,
            format!("reading autosave log: {e}"),
        )
    })?;
    let loaded = crate::io::package::load_package_with(assembly, base_package)?;
    let mut engine = loaded.engine;
    if let Some(doc) = engine.document() {
        if !log.doc_id.is_nil() && doc.id().as_uuid() != log.doc_id {
            return Err(EngineError::new(
                ErrorCode::BadTarget,
                format!(
                    "autosave BASE document id {} does not match the package",
                    log.doc_id
                ),
            ));
        }
    }
    for (id, kind, bytes) in &log.assets {
        // Session-captured values re-enter under their recorded ids; an id
        // already present (base package carried it) is left as-is.
        let _ = engine.insert_loaded_asset(*id, bytes.clone(), *kind);
    }
    let known: std::collections::HashSet<Uuid> = engine.journal().iter().map(|e| e.id).collect();
    let mut replayed = 0usize;
    for env in &log.commands {
        if known.contains(&env.id) {
            continue;
        }
        engine.apply_envelope(env.clone()).map_err(|e| {
            EngineError::new(
                e.code,
                format!(
                    "autosave replay failed at command {} ({}): {}",
                    replayed, env.id, e.message
                ),
            )
        })?;
        replayed += 1;
    }
    Ok((engine, log, replayed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::kind_impl::EngineAssembly;
    use serde_json::json;

    fn assembly() -> EngineAssembly {
        // Post-fill-swap fixture: core's reference FillKind registered
        // explicitly (production fill is tools-side).
        crate::engine::kinds::register(EngineAssembly::new())
            .with_kind(std::sync::Arc::new(crate::engine::kinds::FillKind))
    }

    fn work(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("appname-autosave-{name}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn doc_new_json(n: u32) -> String {
        json!({
            "schema": "appname.command.v1",
            "id": format!("a5000000-0000-4000-8000-0000000{n:05x}"),
            "kind": "doc.new",
            "params": {"width": 128, "height": 128, "background": "white"},
        })
        .to_string()
    }

    fn fill_json(n: u32, layer: Uuid, v: f64) -> String {
        json!({
            "schema": "appname.command.v1",
            "id": format!("a5000000-0000-4000-8000-0001{n:08x}"),
            "kind": "fill",
            "target": {"layerId": layer.to_string()},
            "params": {"content": "color",
                        "color": {"model": "rgb", "comps": [v, 0.25, 1.0 - v, 0.0]}},
        })
        .to_string()
    }

    /// Base engine + saved package + a live session that applied `extra`
    /// fills with the log appended per command.
    fn session(dir: &Path, extra: u32) -> (Engine, PathBuf, PathBuf) {
        let mut e = assembly().build();
        e.apply_json(&doc_new_json(1)).unwrap();
        let base = dir.join("base.appname");
        crate::io::package::save_package(&e, &base).unwrap();
        let doc_id = e.document().unwrap().id().as_uuid();
        let log_path = dir.join("autosave.log");
        let mut log = Autosave::create(&log_path, doc_id).unwrap();
        let layer = e.document().unwrap().z_order_walk()[0].as_uuid();
        for i in 0..extra {
            e.apply_json(&fill_json(
                i,
                layer,
                (i as f64 + 1.0) / (extra as f64 + 1.0),
            ))
            .unwrap();
            let env = e.journal().last().unwrap().clone();
            log.append_command(&env).unwrap();
            log.checkpoint(Some(env.id)).unwrap();
        }
        (e, base, log_path)
    }

    #[test]
    fn recover_replays_to_last_journaled_command() {
        let dir = work("recover");
        let (live, base, log) = session(&dir, 3);
        let (recovered, parsed, replayed) = recover(&assembly(), &base, &log).unwrap();
        assert_eq!(replayed, 3);
        assert!(!parsed.torn);
        assert_eq!(
            recovered.state_hash(),
            live.state_hash(),
            "recovered == live at kill point"
        );
        assert_eq!(recovered.journal().len(), live.journal().len());
    }

    #[test]
    fn torn_tail_costs_the_tail_never_the_document() {
        let dir = work("torn");
        let (live, base, log) = session(&dir, 3);
        // Append garbage: a truncated header.
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
            f.write_all(&[0xde, 0xad, 0xbe]).unwrap();
        }
        let (recovered, parsed, replayed) = recover(&assembly(), &base, &log).unwrap();
        assert!(parsed.torn);
        assert_eq!(replayed, 3, "the valid prefix replays in full");
        assert_eq!(recovered.state_hash(), live.state_hash());
    }

    #[test]
    fn corrupt_middle_record_truncates_at_that_record() {
        let dir = work("corrupt");
        let (_live, base, log) = session(&dir, 3);
        // Flip one byte inside the SECOND command frame's payload: CRC
        // mismatch truncates there, leaving exactly one replayable command.
        let mut bytes = std::fs::read(&log).unwrap();
        // Frame 0 = BASE. Walk to frame 2 (first frame after BASE+cmd1+cp1
        // is cmd2) and flip a payload byte.
        let mut pos = 0usize;
        let mut frame_no = 0;
        loop {
            let len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
            if frame_no == 3 {
                bytes[pos + 8 + len / 2] ^= 0xff;
                break;
            }
            pos += 8 + len;
            frame_no += 1;
        }
        std::fs::write(&log, &bytes).unwrap();
        let (recovered, parsed, replayed) = recover(&assembly(), &base, &log).unwrap();
        assert!(parsed.torn);
        assert_eq!(
            replayed, 1,
            "only the command before the corrupt frame replays"
        );
        assert_eq!(recovered.journal().len(), 2, "doc.new + 1 fill");
    }

    #[test]
    fn asset_frames_carry_captured_values_for_paste() {
        let dir = work("assets");
        let mut e = assembly().build();
        e.apply_json(&doc_new_json(2)).unwrap();
        let base = dir.join("base.appname");
        crate::io::package::save_package(&e, &base).unwrap();
        let doc_id = e.document().unwrap().id().as_uuid();
        let log_path = dir.join("autosave.log");
        let mut log = Autosave::create(&log_path, doc_id).unwrap();
        // Session-registered raster asset + a paste referencing it: both
        // ride the journal (asset frame first — §9.6 rule 2 ordering).
        let png = {
            let mut out = Vec::new();
            let mut enc = png::Encoder::new(&mut out, 2, 2);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&[0u8, 255, 0, 255].repeat(4)).unwrap();
            drop(wr);
            out
        };
        let asset = e.add_raster_asset(png.clone()).unwrap();
        log.append_asset(asset, crate::engine::assets::AssetKind::RasterImage, &png)
            .unwrap();
        let paste = json!({
            "schema": "appname.command.v1",
            "id": "a5000000-0000-4000-8000-000000000777",
            "kind": "edit.paste",
            "params": {"assetId": asset.to_string(), "at": [0, 0],
                        "newLayerId": "a5000000-0000-4000-8000-000000000778"},
        })
        .to_string();
        e.apply_json(&paste).unwrap();
        let env = e.journal().last().unwrap().clone();
        log.append_command(&env).unwrap();
        log.checkpoint(Some(env.id)).unwrap();

        let (recovered, _parsed, replayed) = recover(&assembly(), &base, &log_path).unwrap();
        assert_eq!(replayed, 1);
        assert_eq!(
            recovered.state_hash(),
            e.state_hash(),
            "paste replays via the asset frame"
        );
    }

    /// Fix-round M5 regression: the †-position form of `history.goto`
    /// journals as the equivalent undo/redo run (nothing appended to the
    /// ENGINE journal), so the CLI appends the APPLIED envelope itself to
    /// the §7.8 log — §10.6 crash replay must re-run the goto and land on
    /// the rewound state (the old CLI appended a duplicate of the previous
    /// envelope and the goto was lost).
    #[test]
    fn goto_position_rides_the_log_and_crash_replay_reproduces_it() {
        let dir = work("goto");
        let mut e = assembly().build();
        e.apply_json(&doc_new_json(7)).unwrap();
        let base = dir.join("base.appname");
        crate::io::package::save_package(&e, &base).unwrap();
        let doc_id = e.document().unwrap().id().as_uuid();
        let log_path = dir.join("autosave.log");
        let mut log = Autosave::create(&log_path, doc_id).unwrap();
        let layer = e.document().unwrap().z_order_walk()[0].as_uuid();
        for i in 0..2 {
            let raw = fill_json(100 + i, layer, (i as f64 + 1.0) / 3.0);
            e.apply_json(&raw).unwrap();
            let env = Envelope::from_json(&raw).unwrap();
            log.append_command(&env).unwrap();
            log.checkpoint(Some(env.id)).unwrap();
        }
        // position 2 = doc.new + first fill (§40.8 stack coordinates).
        let goto_raw = serde_json::json!({
            "schema": "appname.command.v1",
            "id": "a5000000-0000-4000-8000-000000000770",
            "kind": "history.goto",
            "params": {"position": 2},
        })
        .to_string();
        e.apply_json(&goto_raw).unwrap();
        assert_eq!(e.journal().len(), 3, "position form appends nothing (†)");
        assert_eq!(e.journal_cursor(), 2, "the run rewound one step");
        let env = Envelope::from_json(&goto_raw).unwrap();
        log.append_command(&env).unwrap();
        log.checkpoint(Some(env.id)).unwrap();

        let (recovered, parsed, replayed) = recover(&assembly(), &base, &log_path).unwrap();
        assert!(!parsed.torn);
        assert_eq!(replayed, 3, "two fills + the goto run");
        assert_eq!(
            recovered.state_hash(),
            e.state_hash(),
            "M5: crash replay reproduces the post-goto state (§10.6)"
        );
        assert_eq!(recovered.journal_cursor(), e.journal_cursor());
        assert_eq!(recovered.journal().len(), e.journal().len());
    }

    #[test]
    fn rebase_truncates_to_a_fresh_base() {
        let dir = work("rebase");
        let (live, base, _log_path) = session(&dir, 2);
        // Simulate the post-save fold: save the package again, rebase.
        let folded = dir.join("folded.appname");
        crate::io::package::save_package(&live, &folded).unwrap();
        let doc_id = live.document().unwrap().id().as_uuid();
        let mut log = Autosave::create(&dir.join("re.log"), doc_id).unwrap();
        log.append_command(live.journal().last().unwrap()).unwrap();
        log.rebase().unwrap();
        let parsed = read_log(log.path()).unwrap();
        assert!(parsed.commands.is_empty(), "rebased log carries only BASE");
        assert!(!parsed.torn);
        // Recovery over the folded package replays nothing and matches.
        let (recovered, _p, replayed) = recover(&assembly(), &folded, log.path()).unwrap();
        assert_eq!(replayed, 0);
        assert_eq!(recovered.state_hash(), live.state_hash());
        let _ = base;
    }
}
