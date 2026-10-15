// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §7 (native file format), §7.4 (tile chunks), §7.6 (atomic
//! saves), §7.8 (journal), §8.2 (PNG codec boundary); DD ⑩; plan §2.1 rows
//! "`.appname` package v1" + "PNG import/export".
//!
//! The IO boundary of the headless kernel: the `.appname` ZIP package
//! (mimetype-first, manifest, document.json, §7.4 tile trees, §7.8 journal
//! entry), and the PNG import/export boundary with its documented CP-0.1
//! color assumption (see [`png_io`]).
//!
//! Everything here reads UNTRUSTED input with typed errors — no panics on
//! malformed packages (the corpus's D-series expectations are the reference
//! bar); parsing hazards are bounded up front (§7.4 size gates, §9.6-style
//! caps).

pub mod autosave;
pub mod integrity;
pub mod package;
pub mod png_io;
pub mod scratch;
pub mod srgb;
pub mod tile_chunk;

use crate::engine::error::{EngineError, ErrorCode};

pub(crate) fn read_err(context: &str, e: impl std::fmt::Display) -> EngineError {
    EngineError::new(ErrorCode::IoReadFailed, format!("{context}: {e}"))
}

pub(crate) fn write_err(context: &str, e: impl std::fmt::Display) -> EngineError {
    EngineError::new(ErrorCode::IoWriteFailed, format!("{context}: {e}"))
}
