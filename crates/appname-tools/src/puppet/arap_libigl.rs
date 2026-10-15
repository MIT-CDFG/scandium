// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The ELECTED production ARAP global-step backend: libigl consumed via
//! the O-INT-ruled cpp-shim shape (reports/INTEGRATION-log.md
//! 2026-08-17 ruling over deps/libigl.yaml — "file-level MPL compliance
//! via the cpp shim from the pinned external checkout", the census
//! obligations column's own arm; riders honored: (1) the Eigen dep
//! record deps/eigen.yaml (main @ 19e3c11) — sanctioned shape enforced
//! by the sys crate's build script: `-DEIGEN_MPL2_ONLY` REQUIRED, the
//! `unsupported/` tree PATH-EXCLUDED from the checkout itself (D1
//! fix-round gate: the build hard-fails if the fetch tooling did not
//! strip it), no BLAS/LAPACK backends; (2) the copyleft-exclusion
//! guard — the shim's include closure references
//! `igl/min_quad_with_fixed.h` ONLY, a member of the record's 127-file
//! ARAP closure inventory, grep-proven by the in-tree guard test and
//! the build-script include check; (3) MLS = own (`appname-algo`,
//! never libigl).
//!
//! D2 restructure (O-INT §5 fix-round ruling): ES §1.3 [NORMATIVE]
//! confines `unsafe` to `*-sys` crates — the FFI boundary lives in
//! `appname-arap-sys` (shape-sys precedent: safe value API, all unsafe
//! confined there, shim TU + build gate co-located). This module is a
//! SAFE adapter: it implements [`ArapGlobalSolver`] over
//! `appname_arap_sys::MqwfFactorization` and contains no unsafe code.
//!
//! CONSUMPTION SHAPE (recorded): the §45.5 solver semantics (local
//! rotation fit, rotation pins, rigidity smoothing, convergence rule)
//! are spec-side Rust in [`super::arap`]; what libigl provides is the
//! ARAP closure's own sparse constrained-quadratic solve spine —
//! `igl::min_quad_with_fixed` — factored once per mesh + pin topology
//! (§18.4 factor-once), back-substituted per iteration. Pinned-vertex
//! elimination happens Rust-side (hard constraints per §18.4); the shim
//! receives the reduced SPD system. Flagged in the lane log for
//! O-INT's merge-time ratification (wholesale-vs-solve-spine reading).

use appname_arap_sys::MqwfFactorization;

use super::arap::{ArapError, ArapGlobalSolver};

/// The libigl-backed [`ArapGlobalSolver`] (safe adapter, D2 shape).
#[derive(Default)]
pub struct LibiglGlobalSolver {
    factored: Option<MqwfFactorization>,
}

impl ArapGlobalSolver for LibiglGlobalSolver {
    fn factor(&mut self, n_free: usize, triplets: &[(u32, u32, f64)]) -> Result<(), ArapError> {
        self.factored = None;
        let f = MqwfFactorization::factor(n_free, triplets).ok_or(ArapError::SolverFailed(
            "min_quad_with_fixed precompute failed (singular or ABI-range system)",
        ))?;
        self.factored = Some(f);
        Ok(())
    }

    fn solve2(&mut self, bx: &[f64], by: &[f64]) -> Result<(Vec<f64>, Vec<f64>), ArapError> {
        let f = self
            .factored
            .as_ref()
            .ok_or(ArapError::SolverFailed("solve before factor"))?;
        if bx.len() != f.dim() || by.len() != f.dim() {
            return Err(ArapError::SolverFailed("rhs length mismatch"));
        }
        let x = f
            .solve(bx)
            .ok_or(ArapError::SolverFailed("min_quad_with_fixed solve failed"))?;
        let y = f
            .solve(by)
            .ok_or(ArapError::SolverFailed("min_quad_with_fixed solve failed"))?;
        Ok((x, y))
    }
}
