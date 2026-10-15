// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Implemented from the published method (P. Pérez, M. Gangnet, A. Blake,
// "Poisson Image Editing", SIGGRAPH 2003 — census paper key perez03),
// following the masked-grid Jacobi/SOR/multigrid solver structure the
// census row records for fpie (B1-fpie.yaml, MIT at pin c33f7d67 — the
// repo is pin-recorded, not staged; NO fpie code was available to or
// copied by this port). OpenCV seamlessClone (Apache-2.0, census row
// B1-opencv-seamless-cloning.yaml) is the behavior REFERENCE for the
// normal/mixed clone semantics — no OpenCV code consulted or copied; its
// oracle role is exercised through the analytic tests below.

//! Spec: ES §45.1 (heal engine: MVC membrane interactive + additive
//! multigrid Poisson commit), §21.1–§21.3, §45.9; ALGORITHMS.md §4
//! `healing` primary `mvc_membrane_interactive_plus_multigrid_poisson_commit`
//! (level L1, basis [farbman09, perez03], `cleared_by: [US6856705B2,
//! US6587592B2, US6985263B2]` per the contract); INTEGRATION-NOTES §6
//! order 5.
//!
//! The healing COMMIT core: gradient-domain blending over a masked region
//! with Dirichlet boundary from the target —
//!
//! * guidance fields: source gradients (seamless/normal clone) or
//!   mixed gradients (per-edge larger magnitude of source/target — the
//!   "mixed clone" semantics);
//! * solvers: Jacobi, SOR (Gauss–Seidel with over-relaxation), and a
//!   multigrid V-cycle over the residual equation — the §45.1 "multigrid
//!   Poisson commit" path.
//!
//! The MVC membrane INTERACTIVE tier (farbman09) is the CP-1.0 heal-tool
//! wave, not this item; this module is the commit-quality solve both tiers
//! share. The register's gated `texture_contrast_variant` (US7605821B1,
//! gate 2028-03-17) is NOT implemented — no texture/contrast matching of
//! any kind exists here, values come strictly from the perez03 blend.
//!
//! Determinism: fixed sweep order (row-major forward), fixed iteration
//! counts, no threading; fixed-seed golden in tests.

use crate::plane::Plane;

/// Guidance-field selection (perez03 §3/§4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guidance {
    /// Source gradients everywhere (seamless / "normal" clone).
    Seamless,
    /// Per-edge larger-magnitude gradient of source vs target
    /// ("mixed gradients", perez03 eq. 13 — transparent-clone semantics).
    Mixed,
}

/// Solver selection for the masked Poisson system.
#[derive(Clone, Copy, Debug)]
pub enum Solver {
    /// Plain Jacobi sweeps.
    Jacobi {
        /// Sweep count.
        iterations: usize,
    },
    /// Gauss–Seidel with over-relaxation factor `omega` (1.0 = plain GS).
    Sor {
        /// Sweep count.
        iterations: usize,
        /// Over-relaxation factor in (0, 2).
        omega: f64,
    },
    /// Multigrid V-cycles over the residual equation (the commit path).
    Multigrid {
        /// V-cycle count.
        cycles: usize,
        /// Pre-/post-smoothing sweeps per level.
        smooth: usize,
    },
}

const FOUR: [(isize, isize); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];

/// The masked Poisson system in residual-free form: interior unknowns with
/// per-pixel right-hand side `b_p = Σ_q v_pq + Σ_{q Dirichlet} f_q`.
struct System {
    w: usize,
    h: usize,
    /// true = interior unknown.
    interior: Vec<bool>,
    /// Right-hand side per pixel (0 outside interior).
    b: Vec<f64>,
}

impl System {
    /// Assemble from target/source/mask (mask > 0.5 = heal region; region
    /// pixels on the image edge are treated as Dirichlet = target).
    fn assemble(target: &Plane, source: &Plane, mask: &Plane, guidance: Guidance) -> System {
        let w = target.w;
        let h = target.h;
        let mut interior = vec![false; w * h];
        for y in 1..h.saturating_sub(1) {
            for x in 1..w.saturating_sub(1) {
                interior[y * w + x] = mask.data[y * w + x] > 0.5;
            }
        }
        let mut b = vec![0f64; w * h];
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if !interior[i] {
                    continue;
                }
                let mut rhs = 0.0f64;
                for (dx, dy) in FOUR {
                    let xx = (x as isize + dx) as usize;
                    let yy = (y as isize + dy) as usize;
                    let j = yy * w + xx;
                    // Guidance along the edge p -> q.
                    let gs = source.data[i] as f64 - source.data[j] as f64;
                    let v = match guidance {
                        Guidance::Seamless => gs,
                        Guidance::Mixed => {
                            let gt = target.data[i] as f64 - target.data[j] as f64;
                            if gt.abs() > gs.abs() {
                                gt
                            } else {
                                gs
                            }
                        }
                    };
                    rhs += v;
                    if !interior[j] {
                        rhs += target.data[j] as f64; // Dirichlet neighbour
                    }
                }
                b[i] = rhs;
            }
        }
        System { w, h, interior, b }
    }

    /// One Jacobi sweep: `f'_p = (b_p + Σ_{q interior} f_q) / 4`.
    fn jacobi_sweep(&self, f: &[f64], out: &mut [f64]) {
        out.copy_from_slice(f);
        for y in 0..self.h {
            for x in 0..self.w {
                let i = y * self.w + x;
                if !self.interior[i] {
                    continue;
                }
                let mut s = self.b[i];
                for (dx, dy) in FOUR {
                    let j = (y as isize + dy) as usize * self.w + (x as isize + dx) as usize;
                    if self.interior[j] {
                        s += f[j];
                    }
                }
                out[i] = s / 4.0;
            }
        }
    }

    /// One SOR sweep in place (row-major forward order — fixed for
    /// determinism).
    fn sor_sweep(&self, f: &mut [f64], omega: f64) {
        for y in 0..self.h {
            for x in 0..self.w {
                let i = y * self.w + x;
                if !self.interior[i] {
                    continue;
                }
                let mut s = self.b[i];
                for (dx, dy) in FOUR {
                    let j = (y as isize + dy) as usize * self.w + (x as isize + dx) as usize;
                    if self.interior[j] {
                        s += f[j];
                    }
                }
                let gs = s / 4.0;
                f[i] += omega * (gs - f[i]);
            }
        }
    }

    /// Residual `r = b - A f` over the interior.
    fn residual(&self, f: &[f64], r: &mut [f64]) {
        r.iter_mut().for_each(|v| *v = 0.0);
        for y in 0..self.h {
            for x in 0..self.w {
                let i = y * self.w + x;
                if !self.interior[i] {
                    continue;
                }
                let mut s = self.b[i] - 4.0 * f[i];
                for (dx, dy) in FOUR {
                    let j = (y as isize + dy) as usize * self.w + (x as isize + dx) as usize;
                    if self.interior[j] {
                        s += f[j];
                    }
                }
                r[i] = s;
            }
        }
    }

    /// Max |residual| (convergence evidence for tests).
    fn residual_inf_norm(&self, f: &[f64]) -> f64 {
        let mut r = vec![0f64; f.len()];
        self.residual(f, &mut r);
        r.iter().fold(0.0f64, |m, v| m.max(v.abs()))
    }
}

/// Homogeneous coarse system for the residual equation (Dirichlet zero on
/// non-interior): interior mask restricted 2x1x... by "any fine interior".
fn coarsen_interior(interior: &[bool], w: usize, h: usize) -> (Vec<bool>, usize, usize) {
    let cw = w.div_ceil(2).max(1);
    let ch = h.div_ceil(2).max(1);
    let mut ci = vec![false; cw * ch];
    for y in 0..h {
        for x in 0..w {
            if interior[y * w + x] {
                let cx = x / 2;
                let cy = y / 2;
                // Coarse border stays Dirichlet (the smoother's stencil
                // never indexes off-grid); the fine level's own smoothing
                // covers the pixels this leaves uncorrected.
                if cx > 0 && cy > 0 && cx + 1 < cw && cy + 1 < ch {
                    ci[cy * cw + cx] = true;
                }
            }
        }
    }
    (ci, cw, ch)
}

/// Multigrid V-cycle on the residual equation of `sys` (recursive).
struct MgLevel {
    sys: System,
}

impl MgLevel {
    fn vcycle(&self, f: &mut [f64], smooth: usize, depth: usize) {
        let sys = &self.sys;
        for _ in 0..smooth {
            sys.sor_sweep(f, 1.0);
        }
        let interior_count = sys.interior.iter().filter(|&&b| b).count();
        if depth == 0 || sys.w <= 4 || sys.h <= 4 || interior_count <= 16 {
            for _ in 0..60 {
                sys.sor_sweep(f, 1.0);
            }
            return;
        }
        // Residual restriction (2x2 box average onto the coarse grid).
        let mut r = vec![0f64; sys.w * sys.h];
        sys.residual(f, &mut r);
        let (ci, cw, ch) = coarsen_interior(&sys.interior, sys.w, sys.h);
        let mut cb = vec![0f64; cw * ch];
        let mut cnt = vec![0u32; cw * ch];
        for y in 0..sys.h {
            for x in 0..sys.w {
                let c = (y / 2) * cw + x / 2;
                cb[c] += r[y * sys.w + x];
                cnt[c] += 1;
            }
        }
        for i in 0..cw * ch {
            if cnt[i] > 0 {
                // Coarse-grid correction scaling. Prolongation `P` is
                // piecewise-constant injection (one coarse value fills the
                // fine 2x2 block); the adjoint restriction is `Pᵀ` = SUM
                // over the block. The Galerkin coarse operator `Pᵀ A P` of
                // the 5-point stencil {4,-1,-1,-1,-1} is {8,-2,-2,-2,-2} =
                // 2·{4,-1,-1,-1,-1}, i.e. twice the rediscretized stencil
                // this level uses. Matching the Galerkin correction
                // `P (2A)⁻¹ Pᵀ r` therefore needs RHS = (1/2)·Pᵀr =
                // (1/2)·Σ_block r = mean·2. (The prior ×4 applied DOUBLE
                // the Galerkin correction — an over-relaxation past the
                // two-grid stability limit: it stayed in-basin for a
                // rectangular full-region mask but diverged on any
                // non-rectangular Ω, e.g. a brush disk — CRITICAL-1.)
                cb[i] = cb[i] / cnt[i] as f64 * 2.0;
            }
            if !ci[i] {
                cb[i] = 0.0;
            }
        }
        let coarse = MgLevel {
            sys: System {
                w: cw,
                h: ch,
                interior: ci,
                b: cb,
            },
        };
        let mut ce = vec![0f64; cw * ch];
        coarse.vcycle(&mut ce, smooth, depth - 1);
        // Prolong (nearest) and correct.
        for y in 0..sys.h {
            for x in 0..sys.w {
                let i = y * sys.w + x;
                if sys.interior[i] {
                    f[i] += ce[(y / 2) * cw + x / 2];
                }
            }
        }
        for _ in 0..smooth {
            sys.sor_sweep(f, 1.0);
        }
    }
}

/// Result of a Poisson heal: the blended plane plus solver evidence.
pub struct HealOutcome {
    /// Blended output plane.
    pub plane: Plane,
    /// Final max-|residual| of the linear system (solver evidence).
    pub residual: f64,
}

/// Gradient-domain heal/clone of `source` into `target` over `mask`
/// (perez03; the §45.1 commit path). Returns the blended plane + residual.
pub fn poisson_heal(
    target: &Plane,
    source: &Plane,
    mask: &Plane,
    guidance: Guidance,
    solver: Solver,
) -> HealOutcome {
    let w = target.w;
    let h = target.h;
    assert!(
        source.w == w && source.h == h && mask.w == w && mask.h == h,
        "poisson_heal: dimension mismatch"
    );
    let sys = System::assemble(target, source, mask, guidance);

    // Initial guess: target values (identity outside, warm start inside).
    let mut f: Vec<f64> = target.data.iter().map(|&v| v as f64).collect();
    for (i, fv) in f.iter_mut().enumerate().take(w * h) {
        if !sys.interior[i] {
            *fv = 0.0; // unknown vector only carries interior values
        }
    }
    // Re-seed interior with target (good warm start).
    for (i, fv) in f.iter_mut().enumerate().take(w * h) {
        if sys.interior[i] {
            *fv = target.data[i] as f64;
        }
    }

    match solver {
        Solver::Jacobi { iterations } => {
            let mut tmp = f.clone();
            for _ in 0..iterations {
                sys.jacobi_sweep(&f, &mut tmp);
                std::mem::swap(&mut f, &mut tmp);
            }
        }
        Solver::Sor { iterations, omega } => {
            for _ in 0..iterations {
                sys.sor_sweep(&mut f, omega);
            }
        }
        Solver::Multigrid { cycles, smooth } => {
            let level = MgLevel { sys };
            for _ in 0..cycles {
                level.vcycle(&mut f, smooth, 16);
            }
            let residual = level.sys.residual_inf_norm(&f);
            let mut out = target.clone();
            for (i, &fv) in f.iter().enumerate().take(w * h) {
                if level.sys.interior[i] {
                    out.data[i] = fv as f32;
                }
            }
            return HealOutcome {
                plane: out,
                residual,
            };
        }
    }
    let residual = sys.residual_inf_norm(&f);
    let mut out = target.clone();
    for (i, &fv) in f.iter().enumerate().take(w * h) {
        if sys.interior[i] {
            out.data[i] = fv as f32;
        }
    }
    HealOutcome {
        plane: out,
        residual,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::{plane_hash, SplitMix64};

    fn rect_mask(w: usize, h: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> Plane {
        let mut m = Plane::new(w, h);
        for y in y0..y1 {
            for x in x0..x1 {
                m.data[y * w + x] = 1.0;
            }
        }
        m
    }

    fn ramp(w: usize, h: usize) -> Plane {
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                p.data[y * w + x] = 0.1 + 0.8 * x as f32 / (w - 1) as f32;
            }
        }
        p
    }

    #[test]
    fn constant_source_membrane_interpolates_linear_boundary() {
        // Zero-gradient source over a linear target: the membrane solution
        // is the harmonic interpolant of the boundary = the linear function
        // itself.
        let target = ramp(24, 20);
        let source = Plane::from_vec(24, 20, vec![0.5; 24 * 20]);
        let mask = rect_mask(24, 20, 6, 5, 18, 15);
        let out = poisson_heal(
            &target,
            &source,
            &mask,
            Guidance::Seamless,
            Solver::Multigrid {
                cycles: 6,
                smooth: 4,
            },
        );
        for i in 0..target.data.len() {
            assert!(
                (out.plane.data[i] - target.data[i]).abs() < 2e-3,
                "pixel {i}: {} vs {}",
                out.plane.data[i],
                target.data[i]
            );
        }
    }

    #[test]
    fn seamless_clone_oracle_offset_source_is_invisible() {
        // The seamless-clone semantics oracle (the role the census assigns
        // the OpenCV behavior reference): a source that differs from the
        // target by a CONSTANT has identical gradients, so the clone must
        // reproduce the target exactly — the seam vanishes.
        let target = ramp(28, 22);
        let mut source = target.clone();
        for v in source.data.iter_mut() {
            *v += 0.3;
        }
        let mask = rect_mask(28, 22, 8, 6, 20, 16);
        let out = poisson_heal(
            &target,
            &source,
            &mask,
            Guidance::Seamless,
            Solver::Multigrid {
                cycles: 6,
                smooth: 4,
            },
        );
        for i in 0..target.data.len() {
            assert!(
                (out.plane.data[i] - target.data[i]).abs() < 2e-3,
                "pixel {i}: offset clone must be invisible"
            );
        }
    }

    #[test]
    fn interior_gradients_follow_the_source() {
        // Textured source into flat target: away from the boundary the
        // solution's gradients match the source's (gradient-domain
        // fidelity).
        let w = 32;
        let h = 26;
        let target = Plane::from_vec(w, h, vec![0.5; w * h]);
        let mut source = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                source.data[y * w + x] =
                    0.5 + 0.2 * ((x as f32 * 0.7).sin() * (y as f32 * 0.5).cos());
            }
        }
        let mask = rect_mask(w, h, 6, 5, 26, 21);
        let out = poisson_heal(
            &target,
            &source,
            &mask,
            Guidance::Seamless,
            Solver::Multigrid {
                cycles: 8,
                smooth: 4,
            },
        )
        .plane;
        // Deep interior only: the harmonic correction that reconciles the
        // flat target boundary with the textured guidance decays with
        // distance from the boundary but is not zero — its residual slope
        // (~6e-3 near the rim on this fixture) is the least-squares
        // trade-off perez03 defines, not solver error.
        for y in 11..16 {
            for x in 13..20 {
                let i = y * w + x;
                let gx_out = out.data[i + 1] - out.data[i];
                let gx_src = source.data[i + 1] - source.data[i];
                assert!(
                    (gx_out - gx_src).abs() < 1e-2,
                    "gradient mismatch at ({x},{y}): {gx_out} vs {gx_src}"
                );
            }
        }
    }

    #[test]
    fn mixed_gradients_keep_the_target_edge() {
        // Flat source over a target with a strong vertical edge: mixed
        // guidance keeps the edge inside the region; seamless smooths it.
        let w = 30;
        let h = 20;
        let mut target = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                target.data[y * w + x] = if x < w / 2 { 0.2 } else { 0.8 };
            }
        }
        let source = Plane::from_vec(w, h, vec![0.5; w * h]);
        let mask = rect_mask(w, h, 9, 5, 21, 15);
        let mg = Solver::Multigrid {
            cycles: 8,
            smooth: 4,
        };
        let mixed = poisson_heal(&target, &source, &mask, Guidance::Mixed, mg).plane;
        let seamless = poisson_heal(&target, &source, &mask, Guidance::Seamless, mg).plane;
        // Edge step at x = 14->15, y mid.
        let y = 10;
        let step_mixed = (mixed.at(15, y) - mixed.at(14, y)).abs();
        let step_seamless = (seamless.at(15, y) - seamless.at(14, y)).abs();
        assert!(
            step_mixed > 0.4,
            "mixed guidance must keep the edge (step {step_mixed})"
        );
        assert!(
            step_seamless < 0.15,
            "seamless guidance interpolates the edge away (step {step_seamless})"
        );
    }

    #[test]
    fn solvers_agree_and_multigrid_converges_fast() {
        let w = 34;
        let h = 30;
        let mut rng = SplitMix64(555);
        let mut target = Plane::new(w, h);
        let mut source = Plane::new(w, h);
        for i in 0..w * h {
            target.data[i] = (0.3 + 0.4 * rng.next_f64()) as f32;
            source.data[i] = (0.2 + 0.6 * rng.next_f64()) as f32;
        }
        let mask = rect_mask(w, h, 8, 7, 26, 23);
        let mg = poisson_heal(
            &target,
            &source,
            &mask,
            Guidance::Seamless,
            Solver::Multigrid {
                cycles: 10,
                smooth: 4,
            },
        );
        let sor = poisson_heal(
            &target,
            &source,
            &mask,
            Guidance::Seamless,
            Solver::Sor {
                iterations: 4000,
                omega: 1.9,
            },
        );
        let jac = poisson_heal(
            &target,
            &source,
            &mask,
            Guidance::Seamless,
            Solver::Jacobi { iterations: 8000 },
        );
        assert!(mg.residual < 1e-6, "multigrid residual {}", mg.residual);
        assert!(sor.residual < 1e-6, "sor residual {}", sor.residual);
        for i in 0..w * h {
            assert!(
                (mg.plane.data[i] - sor.plane.data[i]).abs() < 1e-4,
                "mg vs sor at {i}"
            );
            assert!(
                (mg.plane.data[i] - jac.plane.data[i]).abs() < 1e-3,
                "mg vs jacobi at {i}"
            );
        }
    }

    #[test]
    fn heal_golden_and_determinism() {
        let w = 32;
        let h = 24;
        let mut rng = SplitMix64(0xF01E);
        let mut target = Plane::new(w, h);
        let mut source = Plane::new(w, h);
        for i in 0..w * h {
            target.data[i] = (0.2 + 0.5 * rng.next_f64()) as f32;
            source.data[i] = (0.3 + 0.5 * rng.next_f64()) as f32;
        }
        let mask = rect_mask(w, h, 7, 6, 25, 18);
        let mg = Solver::Multigrid {
            cycles: 6,
            smooth: 4,
        };
        let a = poisson_heal(&target, &source, &mask, Guidance::Mixed, mg).plane;
        let b = poisson_heal(&target, &source, &mask, Guidance::Mixed, mg).plane;
        assert_eq!(plane_hash(&a.data), plane_hash(&b.data));
        assert_eq!(
            plane_hash(&a.data),
            GOLDEN,
            "golden hash drift — investigate before re-pinning (O-QA rule)"
        );
    }

    // Pinned 2026-08-13 after the behavioral oracles above were green
    // (membrane exactness, invisible offset clone, gradient fidelity,
    // mixed-edge preservation, cross-solver agreement at residual < 1e-6).
    //
    // RE-PINNED 2026-08-19 (v1.0.1, CRITICAL-1 heal fix): the coarse-grid
    // correction scaling was corrected from the divergent ×4 to the
    // Galerkin-consistent ×2 (see `vcycle`). On this rectangular-mask
    // fixture the old scaling did not diverge, so only the cycle-6 iterate
    // shifted; the corrected solver converges to residual 5.2e-6 here and
    // matches a 20 000-sweep SOR reference to 4.8e-5 (bit-exact by cycle
    // 200), i.e. the new pin is the correctly-converged solution. All five
    // behavioral oracles above remained green across the change.
    const GOLDEN: u64 = 3500156738502431988;
}
