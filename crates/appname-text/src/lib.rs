// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-text` — Appname text engine: itemize, shape, break, raster.
//!
//! Spec: ES §23.2 (pipeline, [NORMATIVE] pixel-identical across OSes),
//! §23.1 (run model), Part 44 (edge cases; §44.2 fallback chain); DD ㉒.
//!
//! CP-0.7 BRING-UP STATE: the shaping-pipeline seams ([`pipeline`] — the
//! dep-free itemizer and UAX #9 L2 visual reorder) now have their REAL
//! providers behind them (all authorized by adopted deps/ records,
//! 2026-08-13): [`providers`] (ICU4X compiled-data script/grapheme/
//! line-break + unicode-bidi UAX #9), [`shape`] (HarfBuzz via the
//! `appname-shape-sys` wrapper — DD ①: this crate is
//! `forbid(unsafe_code)`, every unsafe block lives in the wrapper), and
//! [`raster`] (FreeType via freetype-rs, Gray f32 coverage). The full
//! §23/Part 44 engine (layout, styles, FontDb/fallback chain, caret/
//! selection geometry, raster cache) is the CP-1.0 row. Font licensing
//! (§3.10): no font files are vendored in this crate or its tests —
//! spike fixtures build a procedural face (authored bytes, no external
//! font data).
//!
//! Determinism spine (D-48, ES §10): every document mutation is a
//! `appname.command.v1` envelope through the single apply entry — no
//! side-door mutation APIs in this crate, ever.

#![forbid(unsafe_code)]

pub mod cache;
pub mod compose;
pub mod door;
pub mod fontdb;
pub mod layout;
pub mod pipeline;
pub mod providers;
pub mod raster;
pub mod shape;
pub mod style;
pub mod tofu;

pub use door::{
    fill_to_linear_rgba, premultiply_coverage, register, register_with_handle, TextLayoutService,
};

/// Crate name marker exercised by the bootstrap smoke test.
pub const CRATE_NAME: &str = "appname-text";

#[cfg(test)]
mod tests {
    /// Bootstrap smoke test: proves the crate compiles and `cargo test`
    /// exercises it.
    #[test]
    fn bootstrap_smoke() {
        assert_eq!(super::CRATE_NAME, "appname-text");
    }
}
