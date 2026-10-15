// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Clean-room implementation from the published papers ONLY: criminisi04
// (priority-front exemplar fill), ashikhmin01 (coherence candidates),
// hertzmann01 (ANN patch search), kwatra03 (seam compositing). Census key
// `caf_primary_cleanroom` (B1-caf-primary.yaml): verdict none-found BY
// DESIGN — no vendorable primary exists, nothing was fetched, no
// third-party expression is present (INTEGRATION-NOTES §5.4).

//! Spec: ES §45.3 (engine contract: constants, sample-region semantics,
//! transform allowances, color adaptation, seed policy, failure rules,
//! Content-Aware Move mapping) + ES §21.4 (the normative procedure);
//! §10.3 `content_aware_fill` command row; ALGORITHMS.md §3.B/§3.E;
//! dossier `caf-exemplar`.
//!
//! Deterministic exemplar synthesis — the content-aware fill core, also
//! backing content-aware move and the spot-heal exemplar tier.
//!
//! # Stages (§45.3 order)
//!
//! 1. pyramid (÷2 Lanczos-3, floor `min(w,h) ≥ 32`, `draft` 64);
//! 2. coarsest init by BFS boundary diffusion ([`diffuse`]);
//! 3. per level, coarse→fine: PCA-16 descriptors over S + immutable k-d
//!    indexes ([`descriptor`], [`kdtree`]), Criminisi priority front with
//!    coherence + index (+ transform-library) candidates and hard patch
//!    commits ([`synth`]), graph-cut seam relabel on the wave-2
//!    [`crate::graphcut`] engine ([`seam`]), R refine passes;
//! 4. between levels: colors + ownership ×2 nearest, 1-px seam
//!    re-diffusion;
//! 5. output: 2-px smoothstep feather + boundary color adaptation
//!    (decontamination, [`adapt`]).
//!
//! Failure rules per §45.3: starved sample regions (< 64 valid centers at
//! the coarsest level) degrade to boundary diffusion at every level (the
//! command SUCCEEDS; [`FillMethod::DiffusionFallback`] tells the caller
//! to emit `error.retouch.insufficient_samples`); an empty boundary with
//! empty samples fails with [`ExemplarError::SampleRegionEmpty`]
//! (`error.retouch.sample_region_empty`).
//!
//! # Rights invariants (contract `content_aware_fill`; ES §45.3 [LEGAL])
//!
//! The three structural invariants are enforced ARCHITECTURALLY in
//! [`synth`]/[`kdtree`] (no per-pixel best-match store exists; every
//! query's candidate set is rebuilt fresh from immutable indexes; no
//! sampled or perturbed search window exists — the §45.3 seeded streams
//! feed only priority tie-breaks and the PCA sample draw) and are
//! regression-locked by this module's mechanical + behavioral tests. The
//! banned randomized-neighbor-field alternative (register P-8285055,
//! active to 2031-04-09) is not implemented in any form, and no
//! date-gated variant code exists in this crate (D-44).
//!
//! # Determinism (D-48, §16.4 rule 5)
//!
//! Pure single-threaded fixed-order scalar code. Same inputs + params
//! (including `seed`) ⇒ byte-identical output; a different seed changes
//! output only where exact ties exist; candidate evaluation order is
//! provably invisible (total-order argmin — exercised by a test).
//!
//! # Adapter seam (D-48)
//!
//! Consumers (O-TOOLS `content_aware_fill` / `fill{content:
//! "content_aware"}` / spot-heal content-aware mode / the Content-Aware
//! Move tool, all via `appname.command.v1` envelopes; O-RENDER GPU pairs
//! later) adapt tiles to [`crate::plane::Plane`]s and call
//! [`exemplar_fill`]. Content-aware move: relocate the region first,
//! pass the exposed area as `hole`, and build params with
//! [`ExemplarParams::for_content_aware_move`] (§45.3 mapping κ = 1.05 −
//! 0.1·structure, w = color/10). Spot healing's content-aware mode is
//! §45.1: `hole` = the stroke footprint, `sample` = `None` (auto),
//! default quality, stroke-derived seed. The §16.2 selection-coverage
//! lerp stays in the evaluator — NOT here.

pub mod adapt;
pub mod descriptor;
pub mod diffuse;
pub mod kdtree;
pub mod stream;
pub mod synth;

mod seam;

pub use adapt::ColorAdaptation;

use crate::exemplar::descriptor::valid_centers;
use crate::exemplar::diffuse::{bfs_diffuse, rediffuse_seams};
use crate::exemplar::pyramid::{downsample_lanczos3, downsample_mask, upsample_nearest};
use crate::exemplar::stream::level_key;
use crate::exemplar::synth::{
    build_libraries, refine, synthesize_level, Knobs, LevelSynth, SrcRef, NO_OWNER,
};
use crate::plane::Plane;

pub mod pyramid;

/// §21.4 quality presets (`draft` / `default` / `best`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    /// Pyramid floor 64, top-4 leaf re-rank, R = 0, no color adaptation.
    Draft,
    /// Floor 32, top-8, R = 1.
    Default,
    /// Floor 32, top-8, R = 2, seam pass repeated after the final refine.
    Best,
}

/// §45.3 `rotationAdaptation` transform allowance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotationAdaptation {
    /// Identity library only (default).
    None,
    /// θ ∈ {−15°, −7.5°, +7.5°, +15°}.
    Low,
    /// θ = k·15°, k = 1…23.
    Full,
}

/// Parameters of the §45.3 engine (§10.3 command row + §45.3 additions).
#[derive(Clone, Copy, Debug)]
pub struct ExemplarParams {
    /// Quality preset (§45.3 "quality" schema addition; default
    /// `default`).
    pub quality: Quality,
    /// §45.3 seed (0..=u32::MAX, default 0). Feeds ONLY the tie-break and
    /// PCA-sample streams — the pipeline is deterministic by
    /// construction; same seed ⇒ byte-identical output.
    pub seed: u32,
    /// Coherence bias κ (§45.3 default 0.75; CAM "structure" maps here).
    pub kappa: f64,
    /// §45.3 `colorAdaptation` (+ CAM "color" strength mapping).
    pub color_adaptation: ColorAdaptation,
    /// §45.3 `rotationAdaptation`.
    pub rotation: RotationAdaptation,
    /// §45.3 `scale`: s ∈ {2⁻¹, 2⁻⁰·⁵, 2⁺⁰·⁵, 2⁺¹} libraries.
    pub scale: bool,
    /// §45.3 `mirror`: horizontal-mirror copies (compose with θ/s sets).
    pub mirror: bool,
}

impl Default for ExemplarParams {
    fn default() -> Self {
        ExemplarParams {
            quality: Quality::Default,
            seed: 0,
            kappa: 0.75,
            color_adaptation: ColorAdaptation::Global { strength: 1.0 },
            rotation: RotationAdaptation::None,
            scale: false,
            mirror: false,
        }
    }
}

impl ExemplarParams {
    /// §45.3 Content-Aware Move mapping: `structure 1–7` ⇒ κ = 1.05 −
    /// 0.1·structure; `color 0–10` ⇒ adaptation strength w = color/10 on
    /// the `default` (global) fit. Inputs clamp to their §32.3 ranges.
    pub fn for_content_aware_move(structure: u8, color: u8) -> Self {
        let structure = structure.clamp(1, 7);
        let color = color.min(10);
        ExemplarParams {
            kappa: 1.05 - 0.1 * structure as f64,
            color_adaptation: ColorAdaptation::Global {
                strength: color as f32 / 10.0,
            },
            ..ExemplarParams::default()
        }
    }
}

/// Which path produced the pixels (§45.3 failure rule 1 reporting seam:
/// the fallback SUCCEEDS but the caller surfaces
/// `error.retouch.insufficient_samples`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillMethod {
    /// The §45.3 deterministic exemplar synthesis ran.
    Exemplar,
    /// Starved sample region: boundary-diffusion degradation ran.
    DiffusionFallback,
}

/// Fill output.
pub struct ExemplarResult {
    /// Filled channels (same order/dimensions as the input).
    pub planes: Vec<Plane>,
    /// Which §45.3 path produced the hole pixels.
    pub method: FillMethod,
    /// Transform libraries dropped by the §45.3 materialization cap
    /// (summed over levels; callers document drops in the job log).
    pub dropped_libraries: usize,
}

/// Engine errors (§45.3 failure rule 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExemplarError {
    /// Empty boundary and empty samples — nothing can produce pixels
    /// (`APPNAME_ERR_INVALID_ARG` / `error.retouch.sample_region_empty`).
    SampleRegionEmpty,
}

/// Debug payload for the crate's oracle tests (finest-level ownership +
/// provenance). Not part of the adapter seam; fields are read by the
/// cfg(test) suite only.
#[allow(dead_code)]
pub(crate) struct FillDebug {
    pub finest_owner: Vec<u32>,
    pub finest_src: Vec<SrcRef>,
    pub finest_patches: Vec<synth::PatchRec>,
    pub hole: Vec<bool>,
}

/// Chebyshev dilation by `radius` (the §45.3 2-px guard band G).
fn dilate(mask: &[bool], w: usize, h: usize, radius: isize) -> Vec<bool> {
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            if !mask[y * w + x] {
                continue;
            }
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let (xx, yy) = (x as isize + dx, y as isize + dy);
                    if xx >= 0 && yy >= 0 && xx < w as isize && yy < h as isize {
                        out[yy as usize * w + xx as usize] = true;
                    }
                }
            }
        }
    }
    out
}

/// Content-aware fill by deterministic exemplar synthesis (ES §45.3).
///
/// * `planes` — color channels, linear working space (premultiplied when
///   `alpha` is given, per the dossier's alpha rule).
/// * `alpha` — optional coverage; fully transparent pixels (α ≤ 1e−3)
///   are invalid sample candidates.
/// * `hole` — H, coverage ≥ 0.5 (§45.3).
/// * `sample` — optional S restriction (≥ 0.5); `None` = `auto` (canvas
///   minus hole and 2-px guard band). The `rectangular` semantic of
///   §45.3 is caller-side geometry over this same parameter.
///
/// Pure function; byte-exact per (inputs, params) — see module docs.
///
/// # Panics
///
/// Panics on programmer error at the adapter seam (crate convention:
/// contract violations are asserts, not [`ExemplarError`] variants —
/// the error enum is reserved for §45.3's normative runtime failure
/// rules, which adapters must surface to users):
///
/// * `planes` is empty (no channels to fill);
/// * any plane in `planes`, or `alpha`, or `sample`, differs in
///   dimensions from `hole`.
pub fn exemplar_fill(
    planes: &[Plane],
    alpha: Option<&Plane>,
    hole: &Plane,
    sample: Option<&Plane>,
    params: &ExemplarParams,
) -> Result<ExemplarResult, ExemplarError> {
    fill_impl(planes, alpha, hole, sample, params, Knobs::default()).map(|(r, _)| r)
}

/// Test entry: same engine with ablation/permutation knobs + debug info.
#[cfg(test)]
pub(crate) fn fill_with_knobs(
    planes: &[Plane],
    alpha: Option<&Plane>,
    hole: &Plane,
    sample: Option<&Plane>,
    params: &ExemplarParams,
    knobs: Knobs,
) -> Result<(ExemplarResult, FillDebug), ExemplarError> {
    fill_impl(planes, alpha, hole, sample, params, knobs)
}

fn fill_impl(
    planes: &[Plane],
    alpha: Option<&Plane>,
    hole: &Plane,
    sample: Option<&Plane>,
    params: &ExemplarParams,
    knobs: Knobs,
) -> Result<(ExemplarResult, FillDebug), ExemplarError> {
    let w = hole.w;
    let h = hole.h;
    assert!(!planes.is_empty(), "exemplar_fill: no channels");
    for p in planes {
        assert!(p.w == w && p.h == h, "exemplar_fill: dimension mismatch");
    }
    if let Some(a) = alpha {
        assert!(a.w == w && a.h == h, "exemplar_fill: alpha mismatch");
    }
    if let Some(s) = sample {
        assert!(s.w == w && s.h == h, "exemplar_fill: sample mismatch");
    }

    let hole_b: Vec<bool> = hole.data.iter().map(|&v| v >= 0.5).collect();
    let debug_empty = FillDebug {
        finest_owner: vec![NO_OWNER; w * h],
        finest_src: vec![SrcRef::NONE; w * h],
        finest_patches: Vec::new(),
        hole: hole_b.clone(),
    };
    if !hole_b.iter().any(|&b| b) {
        return Ok((
            ExemplarResult {
                planes: planes.to_vec(),
                method: FillMethod::Exemplar,
                dropped_libraries: 0,
            },
            debug_empty,
        ));
    }
    // §45.3 failure rule 2: an all-hole canvas has neither a diffusion
    // boundary nor any sample pixel.
    let boundary_exists = (0..w * h).any(|i| {
        if hole_b[i] {
            return false;
        }
        let (x, y) = (i % w, i / w);
        [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)]
            .iter()
            .any(|&(dx, dy)| {
                let (xx, yy) = (x as isize + dx, y as isize + dy);
                xx >= 0
                    && yy >= 0
                    && xx < w as isize
                    && yy < h as isize
                    && hole_b[yy as usize * w + xx as usize]
            })
    });
    if !boundary_exists {
        return Err(ExemplarError::SampleRegionEmpty);
    }

    // Finest sample region: S = (user S | canvas) ∖ G ∖ transparent.
    let mut s0: Vec<bool> = match sample {
        Some(s) => s.data.iter().map(|&v| v >= 0.5).collect(),
        None => vec![true; w * h],
    };
    let guard = dilate(&hole_b, w, h, 2);
    for i in 0..w * h {
        if guard[i] {
            s0[i] = false;
        }
        if let Some(a) = alpha {
            if a.data[i] <= 1e-3 {
                s0[i] = false;
            }
        }
    }

    // Pyramid (level 0 = finest).
    let floor = match params.quality {
        Quality::Draft => 64usize,
        _ => 32,
    };
    struct Level {
        planes: Vec<Plane>,
        hole: Vec<bool>,
        s: Vec<bool>,
        w: usize,
        h: usize,
    }
    let mut levels: Vec<Level> = vec![Level {
        planes: planes.to_vec(),
        hole: hole_b.clone(),
        s: s0.clone(),
        w,
        h,
    }];
    loop {
        let last = levels.last().unwrap();
        let (nw, nh) = (last.w.div_ceil(2), last.h.div_ceil(2));
        if nw.min(nh) < floor {
            break;
        }
        let nplanes: Vec<Plane> = last.planes.iter().map(downsample_lanczos3).collect();
        let (nhole, _, _) = downsample_mask(&last.hole, last.w, last.h);
        let (mut ns, _, _) = downsample_mask(&last.s, last.w, last.h);
        // Re-assert the 2-px guard band at this level (S must keep clear
        // of the level's own hole).
        let g = dilate(&nhole, nw, nh, 2);
        for i in 0..nw * nh {
            if g[i] {
                ns[i] = false;
            }
        }
        levels.push(Level {
            planes: nplanes,
            hole: nhole,
            s: ns,
            w: nw,
            h: nh,
        });
    }

    // §45.3 minimum viable sample: N_min = 64 valid centers at the
    // coarsest level.
    let coarsest = levels.len() - 1;
    let starved =
        valid_centers(&levels[coarsest].s, levels[coarsest].w, levels[coarsest].h).len() < 64;

    let original_finest = planes.to_vec();
    if starved {
        // Failure rule 1: boundary diffusion at every level, coarse→fine,
        // upsampled between levels; feather; NO color adaptation.
        for l in (0..levels.len()).rev() {
            if l < coarsest {
                let (cw, chh) = (levels[l + 1].w, levels[l + 1].h);
                let coarse: Vec<Vec<f32>> = levels[l + 1]
                    .planes
                    .iter()
                    .map(|p| p.data.clone())
                    .collect();
                let lw = levels[l].w;
                let lh = levels[l].h;
                for (c, data) in coarse.iter().enumerate() {
                    let up = upsample_nearest(data, cw, chh, lw, lh);
                    for (i, &v) in up.iter().enumerate() {
                        if levels[l].hole[i] {
                            levels[l].planes[c].data[i] = v;
                        }
                    }
                }
            }
            let lev = &mut levels[l];
            let hole_l = lev.hole.clone();
            bfs_diffuse(&mut lev.planes, &hole_l, lev.w, lev.h);
        }
        let mut out = levels.swap_remove(0).planes;
        adapt::apply_output(
            &mut out,
            &original_finest,
            &hole_b,
            w,
            h,
            ColorAdaptation::None,
        );
        return Ok((
            ExemplarResult {
                planes: out,
                method: FillMethod::DiffusionFallback,
                dropped_libraries: 0,
            },
            debug_empty,
        ));
    }

    // Transform allowance sets (§45.3 tables).
    let rotations: Vec<f64> = match params.rotation {
        RotationAdaptation::None => Vec::new(),
        RotationAdaptation::Low => vec![-15.0, -7.5, 7.5, 15.0],
        RotationAdaptation::Full => (1..=23).map(|k| 15.0 * k as f64).collect(),
    };
    let scales: Vec<f64> = if params.scale {
        vec![0.5, 2f64.powf(-0.5), 2f64.powf(0.5), 2.0]
    } else {
        Vec::new()
    };
    let (top_l, refine_passes) = match params.quality {
        Quality::Draft => (4usize, 0usize),
        Quality::Default => (8, 1),
        Quality::Best => (8, 2),
    };

    let mut dropped = 0usize;
    let mut prev: Option<(LevelSynth, usize, usize)> = None; // (synth, w, h)
    let mut finest_debug: Option<(LevelSynth, Vec<bool>)> = None;
    for l in (0..levels.len()).rev() {
        // Init the hole: coarsest by diffusion, finer by carry-up.
        if l == coarsest {
            let lev = &mut levels[l];
            let hole_l = lev.hole.clone();
            bfs_diffuse(&mut lev.planes, &hole_l, lev.w, lev.h);
        } else {
            let (psynth, pw, ph) = prev.take().unwrap();
            let coarse: Vec<Vec<f32>> = levels[l + 1]
                .planes
                .iter()
                .map(|p| p.data.clone())
                .collect();
            let lw = levels[l].w;
            let lh = levels[l].h;
            for (c, data) in coarse.iter().enumerate() {
                let up = upsample_nearest(data, pw, ph, lw, lh);
                for (i, &v) in up.iter().enumerate() {
                    if levels[l].hole[i] {
                        levels[l].planes[c].data[i] = v;
                    }
                }
            }
            let owner_up = upsample_nearest(&psynth.owner, pw, ph, lw, lh);
            let lev = &mut levels[l];
            let hole_l = lev.hole.clone();
            rediffuse_seams(&mut lev.planes, &hole_l, &owner_up, lw, lh);
        }

        let key = level_key(params.seed, l as u32);
        let lev = &levels[l];
        let libs = build_libraries(
            &lev.planes,
            &lev.s,
            lev.w,
            lev.h,
            &rotations,
            &scales,
            params.mirror,
            &key,
        );
        dropped += libs.dropped;
        let (lw, lh) = (lev.w, lev.h);
        let hole_l = lev.hole.clone();
        let lev = &mut levels[l];
        let mut synth = synthesize_level(
            &mut lev.planes,
            &hole_l,
            lw,
            lh,
            &libs,
            params.kappa,
            top_l,
            &key,
            knobs,
        );
        if !knobs.no_seam {
            seam::seam_pass(&mut lev.planes, &hole_l, lw, lh, &libs, &mut synth);
        }
        refine(
            &mut lev.planes,
            &hole_l,
            lw,
            lh,
            &libs,
            params.kappa,
            top_l,
            knobs,
            &mut synth,
            refine_passes,
        );
        if params.quality == Quality::Best && !knobs.no_seam {
            seam::seam_pass(&mut lev.planes, &hole_l, lw, lh, &libs, &mut synth);
        }
        if l == 0 {
            finest_debug = Some((synth, hole_l));
        } else {
            prev = Some((synth, lw, lh));
        }
    }

    let (finest_synth, finest_hole) = finest_debug.unwrap();
    let mut out = levels.swap_remove(0).planes;
    // §21.4: draft preset runs no color adaptation.
    let adaptation = if params.quality == Quality::Draft {
        ColorAdaptation::None
    } else {
        params.color_adaptation
    };
    adapt::apply_output(&mut out, &original_finest, &hole_b, w, h, adaptation);
    Ok((
        ExemplarResult {
            planes: out,
            method: FillMethod::Exemplar,
            dropped_libraries: dropped,
        },
        FillDebug {
            finest_owner: finest_synth.owner,
            finest_src: finest_synth.src,
            finest_patches: finest_synth.patches,
            hole: finest_hole,
        },
    ))
}

#[cfg(test)]
mod tests;
