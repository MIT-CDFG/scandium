// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Edge-aware detail family — clean-room from the published papers.
//!
//! Spec: ALGORITHMS.md §14 (14.B [NORMATIVE] selected baseline); dossiers
//! `detail-local-laplacian` + `shadows-highlights`; ES §17.2
//! Shadows/Highlights (`adjustment.shadows_highlights`) and ES §16.3
//! Surface Blur / High Pass rows (§ numbers verified against the current
//! ENGINEERING-SPEC.md text). Contract: `contracts/algorithms/algorithms.yaml`
//! area `edge_aware_detail` — primary `fast_local_laplacian` (L2, basis
//! aubry14 + paris11, `fto_flagship: true`), best `wls` (L2, farbman08),
//! preview `domain_transform` (L2, gastal11), shadows_highlights
//! `guided_filter_split` (L2, he10, `design_around: US9241093B2`).
//!
//! Rights route (census, INTEGRATION-NOTES §6 item 10): the area's author
//! codes are no-grant/none-found (B2-llf-author-code, B2-wls-author-code,
//! B2-domain-transform-author-code) and the B2-opencv-ximgproc census row
//! records that ximgproc holds NO local-Laplacian implementation at the
//! pinned commit — every engine in this directory is therefore implemented
//! STRICTLY from the published papers; no reference codebase was fetched or
//! consulted (INTEGRATION-NOTES §5.4). The only shared code is the crate's
//! own [`crate::fusion`] Laplacian-pyramid substrate (BSD-2-Clause port,
//! notice already carried) reused crate-internally by [`llf`].
//!
//! Modules:
//! - [`guided`] — guided filter (He, Sun, Tang 2010) + the base/detail
//!   split behind Shadows/Highlights (design-around posture documented
//!   there).
//! - [`dtf`] — domain transform (Gastal & Oliveira 2011), NC + RF variants
//!   (dossier selects NC, 3 iterations, as the preview engine).
//! - [`wls`] — weighted-least-squares decomposition (Farbman, Fattal,
//!   Lischinski, Szeliski 2008) on the crate's ichol+CG sparse machinery.
//! - [`llf`] — local Laplacian filtering: paris11 exact per-pixel path
//!   (quality `best`) + aubry14 fast discretized-level path (the primary).
//!
//! Determinism (D-48): pure functions over planar f32 rasters, f64 internal
//! accumulation, single-threaded fixed iteration order, no RNG. All engines
//! validate inputs and return `Result<_, String>`.
//!
//! ADAPTER SEAM (brokered): O-TOOLS adapts tile data into planes, maps the
//! §17.2 sliders onto these engines' parameters (Clarity/Texture -> [`llf`]
//! remap curves; Shadows/Highlights -> [`guided`] split + §17.2 lift
//! curves; Surface Blur / High Pass per §16.3), and lands every mutation
//! through the `appname.command.v1` envelope spine. GPU pairs are O-RENDER
//! parity items against these CPU references.

pub mod dtf;
pub mod guided;
pub mod llf;
pub mod wls;
