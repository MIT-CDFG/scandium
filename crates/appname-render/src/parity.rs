// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §34.3 (GPU/CPU parity — the golden suite on both paths, max
//! abs diff ≤ 1/255 at 8-bit-equivalent + SSIM ≥ 0.999; Dissolve
//! bit-exact against its own baseline on both paths; documented
//! exceptions live in Appendix A), §4.8 (the CPU path is the golden
//! reference), W24 (forced-CPU stays first-class: the CPU backend is
//! UNCONDITIONAL — it exists in every build, needs no adapter, and is
//! the reference the GPU leg is judged against, never the other way
//! around); plan §2.3 "parity gate ≤ 1/255 CPU-vs-GPU on the golden
//! corpus … this gate is yours to keep green forever" (§3.4).
//!
//! The harness is backend-generic: a [`CompositeBackend`] executes the
//! §4.5 `cs_composite_full` contract on one tile. [`CpuBackend`] wraps
//! this crate's kernels (the permanent oracle); the wgpu backend (slice
//! D) implements the same trait over the real pipelines, and the §34.3
//! gate is then `run_parity(&mut gpu, &mut cpu, corpus)` +
//! [`ParityReport::gate`]. Until the GPU leg lands, the harness is
//! proven by self-parity (identical backends → zero diff) and by
//! fault-injection backends (a 2/255 single-pixel error MUST trip the
//! gate — the detector is tested, not assumed).
//!
//! **Corpus.** [`kernel_corpus`] builds the deterministic kernel-level
//! corpus: all 27 modes × source/backdrop tile patterns covering the
//! §4.4/§6.4 regimes — alpha 0 / fractional / 1, dyadic and non-dyadic
//! values, HDR super-brights, gradient ramps, and seeded noise
//! (SplitMix64, fixed seeds — every machine regenerates bit-identical
//! inputs, §34.5 fixture discipline). O-QA's cp05 golden-corpus layer
//! (wave 2) rides on top at the document level; this corpus is the
//! kernel-level floor and stays in-crate so the gate cannot go corpusless.
//!
//! **Metrics.** Max-abs is measured in 8-bit-equivalent units:
//! `255 · max|a − b|` over all channels/pixels (HDR values included
//! verbatim — stricter than a display-referred reading). SSIM is the
//! published Wang–Bovik formula (own implementation): luma via the §4.5
//! compositing weights on clamped-[0,1] un-premultiplied color, 8×8
//! non-overlapping windows, K1 = 0.01, K2 = 0.03, L = 1; the reported
//! value is the minimum window mean per case (conservative).

use appname_core::BlendMode;

use crate::blend::{composite_tile_ctx, DissolveCtx};
use crate::tile_rgba::TileRgba;
use appname_core::TILE;

/// One kernel-level parity case: composite `src` over `dst` with `mode`
/// at `opacity` (Dissolve reads `ctx`).
pub struct KernelCase {
    /// Case label (worst-case reporting).
    pub name: String,
    pub mode: BlendMode,
    pub src: TileRgba,
    pub dst: TileRgba,
    pub opacity: f32,
    pub ctx: DissolveCtx,
    /// §11.2 per-pixel effective coverage E (row-major `TILE·TILE`),
    /// host-computed by the ONE shared routine
    /// ([`appname_core::resolve_effective_coverage`]). `None` ⇒ unmasked.
    /// Every backend multiplies the premultiplied source by this E
    /// ([`masked_source`] / the GPU's `src *= coverage`) BEFORE the
    /// composite — a single correctly-rounded multiply, so the §34.3 gate
    /// on masked cases is exactly as strong as on un-masked ones.
    pub coverage: Option<Vec<f32>>,
}

/// Apply a case's §11.2 coverage to its source — the SAME per-pixel
/// multiply the GPU kernel does (`src *= coverage`), through the shared
/// [`appname_core::apply_coverage`]. Unmasked ⇒ the source unchanged.
/// Every [`CompositeBackend`] routes its source through this so coverage
/// is applied exactly ONE way across the harness.
pub fn masked_source(case: &KernelCase) -> TileRgba {
    let Some(e) = &case.coverage else {
        return case.src.clone();
    };
    let mut s = case.src.clone();
    for y in 0..TILE {
        for x in 0..TILE {
            s.set(
                x,
                y,
                appname_core::apply_coverage(s.get(x, y), e[y * TILE + x]),
            );
        }
    }
    s
}

/// A backend that executes the §4.5 `cs_composite_full` contract on one
/// tile. Implementations MUST be deterministic.
pub trait CompositeBackend {
    /// Backend name for reports (`"cpu"`, `"wgpu-metal"`, …).
    fn name(&self) -> &str;
    /// Composite `case.src` over `case.dst`, returning the result tile.
    fn composite(&mut self, case: &KernelCase) -> TileRgba;
}

/// The permanent CPU reference (§4.8) — unconditional in every build
/// (W24: no adapter, no feature flag, no fallback ladder below it).
pub struct CpuBackend;

impl CompositeBackend for CpuBackend {
    fn name(&self) -> &str {
        "cpu"
    }

    fn composite(&mut self, case: &KernelCase) -> TileRgba {
        let src = masked_source(case);
        let mut acc = case.dst.clone();
        composite_tile_ctx(&mut acc, &src, case.mode, case.opacity, case.ctx);
        acc
    }
}

/// Result of one corpus run between two backends.
#[derive(Clone, Debug)]
pub struct ParityReport {
    /// `255 · max|a − b|` over every case/pixel/channel (Dissolve cases
    /// excluded — they carry the bit-exactness verdict instead).
    pub max_abs_8bit: f32,
    /// The case that produced `max_abs_8bit`.
    pub worst_case: String,
    /// Minimum per-case SSIM (non-Dissolve cases).
    pub min_ssim: f64,
    /// The case that produced `min_ssim`.
    pub worst_ssim_case: String,
    /// Dissolve COVERAGE mismatches (§4.5 [NORMATIVE] determinism): the
    /// pass/fail pattern must be identical across backends — thresholds
    /// and the hardened αs are exact on both paths (multiplication and
    /// comparison are correctly rounded everywhere; only division is
    /// not). Measured bitwise on the alpha channel over a TRANSPARENT
    /// backdrop, where αo ≡ the hardened αs exactly. Whole-tile
    /// cross-path bit-equality is deliberately NOT required — the §34.3
    /// sentence is "bit-exact against its OWN baseline on both paths",
    /// and the Porter-Duff tail's divisions legitimately differ by ULPs
    /// across drivers (Metal run evidence, lane log).
    pub dissolve_coverage_mismatches: usize,
    /// Dissolve own-baseline failures: a backend that returns different
    /// bytes for the same case on a double-run (the §34.3 own-baseline
    /// determinism sentence, executable form).
    pub dissolve_determinism_failures: usize,
    /// Total cases executed.
    pub cases: usize,
}

/// §34.3 tolerance failure.
#[derive(Clone, Debug, PartialEq)]
pub enum ParityFailure {
    /// `max_abs_8bit > 1.0` (the ≤ 1/255 gate), with the offending case.
    MaxAbs { value: f32, case: String },
    /// `min_ssim < 0.999`, with the offending case.
    Ssim { value: f64, case: String },
    /// A Dissolve case's pass/fail pattern differed between backends.
    DissolveCoverage { mismatches: usize },
    /// A backend was not self-deterministic on a Dissolve double-run.
    DissolveDeterminism { failures: usize },
}

impl ParityReport {
    /// Apply the §34.3 tolerance table.
    pub fn gate(&self) -> Result<(), ParityFailure> {
        if self.dissolve_determinism_failures > 0 {
            return Err(ParityFailure::DissolveDeterminism {
                failures: self.dissolve_determinism_failures,
            });
        }
        if self.dissolve_coverage_mismatches > 0 {
            return Err(ParityFailure::DissolveCoverage {
                mismatches: self.dissolve_coverage_mismatches,
            });
        }
        if self.max_abs_8bit > 1.0 {
            return Err(ParityFailure::MaxAbs {
                value: self.max_abs_8bit,
                case: self.worst_case.clone(),
            });
        }
        if self.min_ssim < 0.999 {
            return Err(ParityFailure::Ssim {
                value: self.min_ssim,
                case: self.worst_ssim_case.clone(),
            });
        }
        Ok(())
    }
}

/// Run `corpus` on both backends and compare per the §34.3 tolerance
/// table (module doc). `reference` should be the CPU oracle.
pub fn run_parity(
    candidate: &mut dyn CompositeBackend,
    reference: &mut dyn CompositeBackend,
    corpus: &[KernelCase],
) -> ParityReport {
    let mut report = ParityReport {
        max_abs_8bit: 0.0,
        worst_case: String::new(),
        min_ssim: 1.0,
        worst_ssim_case: String::new(),
        dissolve_coverage_mismatches: 0,
        dissolve_determinism_failures: 0,
        cases: corpus.len(),
    };
    let bits_equal = |a: &TileRgba, b: &TileRgba| {
        a.as_slice()
            .iter()
            .zip(b.as_slice())
            .all(|(x, y)| x.to_bits() == y.to_bits())
    };
    for case in corpus {
        let a = candidate.composite(case);
        let b = reference.composite(case);
        if case.mode == BlendMode::Dissolve {
            // (1) Own-baseline determinism: double-run each backend
            // (to_bits equality — −0.0/+0.0 and NaN handled without
            // argument).
            if !bits_equal(&a, &candidate.composite(case))
                || !bits_equal(&b, &reference.composite(case))
            {
                report.dissolve_determinism_failures += 1;
            }
            // (2) Cross-path coverage: the same case over a TRANSPARENT
            // backdrop makes αo ≡ the hardened αs bitwise on both paths
            // — the §4.5 pattern must match exactly.
            let coverage_case = KernelCase {
                name: format!("{}/coverage", case.name),
                mode: case.mode,
                src: case.src.clone(),
                dst: TileRgba::transparent(),
                opacity: case.opacity,
                ctx: case.ctx,
                coverage: case.coverage.clone(),
            };
            let ca = candidate.composite(&coverage_case);
            let cb = reference.composite(&coverage_case);
            let alpha_equal = ca
                .as_slice()
                .chunks_exact(4)
                .zip(cb.as_slice().chunks_exact(4))
                .all(|(x, y)| x[3].to_bits() == y[3].to_bits());
            if !alpha_equal {
                report.dissolve_coverage_mismatches += 1;
            }
            // (3) The composited tile still meets the standard numeric
            // tolerances (falls through to the shared track below).
        }
        let d = max_abs_8bit(&a, &b);
        if d > report.max_abs_8bit {
            report.max_abs_8bit = d;
            report.worst_case = case.name.clone();
        }
        // Non-finite poison rule (see max_abs_8bit): a NaN SSIM would
        // lose every `<` comparison and vanish — clamp to 0.0 (fail).
        let s = ssim(&a, &b);
        let s = if s.is_finite() { s } else { 0.0 };
        if s < report.min_ssim {
            report.min_ssim = s;
            report.worst_ssim_case = case.name.clone();
        }
    }
    report
}

/// `255 · max|a − b|` over all scalars (8-bit-equivalent units).
///
/// **Non-finite poison rule:** any NaN/∞ difference reports as +∞ and
/// therefore FAILS the gate. Without this, `f32::max` would silently
/// ignore NaN operands (IEEE maxNum semantics) and a NaN-producing
/// backend would gate GREEN — the exact silent-corruption mode the
/// §34.3 gate exists to catch (`nan_poison_fails_the_gate`).
pub fn max_abs_8bit(a: &TileRgba, b: &TileRgba) -> f32 {
    a.as_slice()
        .iter()
        .zip(b.as_slice())
        .map(|(x, y)| {
            let d = (x - y).abs() * 255.0;
            if d.is_finite() {
                d
            } else {
                f32::INFINITY
            }
        })
        .fold(0.0f32, f32::max)
}

/// SSIM (module doc): luma of clamped un-premultiplied color, 8×8
/// non-overlapping windows, K1 = 0.01, K2 = 0.03, L = 1; returns the
/// MINIMUM window value (conservative against localized artifacts,
/// which a global mean would wash out).
pub fn ssim(a: &TileRgba, b: &TileRgba) -> f64 {
    const W: usize = 8;
    const C1: f64 = 0.01 * 0.01;
    const C2: f64 = 0.03 * 0.03;
    let luma = |t: &TileRgba, x: usize, y: usize| -> f64 {
        let p = t.get(x, y);
        let (r, g, bl) = if p[3] > 0.0 {
            (p[0] / p[3], p[1] / p[3], p[2] / p[3])
        } else {
            (0.0, 0.0, 0.0)
        };
        let clamp = |v: f32| v.clamp(0.0, 1.0) as f64;
        0.3 * clamp(r) + 0.59 * clamp(g) + 0.11 * clamp(bl)
    };
    let mut min_ssim = 1.0f64;
    for wy in (0..TILE).step_by(W) {
        for wx in (0..TILE).step_by(W) {
            let n = (W * W) as f64;
            let (mut ma, mut mb) = (0.0f64, 0.0f64);
            for y in wy..wy + W {
                for x in wx..wx + W {
                    ma += luma(a, x, y);
                    mb += luma(b, x, y);
                }
            }
            ma /= n;
            mb /= n;
            let (mut va, mut vb, mut cov) = (0.0f64, 0.0f64, 0.0f64);
            for y in wy..wy + W {
                for x in wx..wx + W {
                    let da = luma(a, x, y) - ma;
                    let db = luma(b, x, y) - mb;
                    va += da * da;
                    vb += db * db;
                    cov += da * db;
                }
            }
            va /= n - 1.0;
            vb /= n - 1.0;
            cov /= n - 1.0;
            let s = ((2.0 * ma * mb + C1) * (2.0 * cov + C2))
                / ((ma * ma + mb * mb + C1) * (va + vb + C2));
            min_ssim = min_ssim.min(s);
        }
    }
    min_ssim
}

// ---------------------------------------------------------------------
// The deterministic kernel corpus (module doc).
// ---------------------------------------------------------------------

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// Deterministic pattern tiles. Every pattern is premultiplied-valid
/// (`rgb ≤ a` except the deliberate HDR patterns, §6.4 super-bright).
fn pattern(kind: &str, seed: u64) -> TileRgba {
    let mut t = TileRgba::transparent();
    let mut rng = seed;
    for y in 0..TILE {
        for x in 0..TILE {
            let v = match kind {
                // Horizontal alpha ramp: a sweeps 0→1; color rides it.
                "alpha-ramp" => {
                    let a = x as f32 / (TILE - 1) as f32;
                    [0.8 * a, 0.5 * a, 0.2 * a, a]
                }
                // Diagonal color gradient at fractional alpha.
                "gradient" => {
                    let u = x as f32 / (TILE - 1) as f32;
                    let w = y as f32 / (TILE - 1) as f32;
                    [0.5 * u, 0.5 * w, 0.5 * (1.0 - u), 0.5]
                }
                // Opaque saturated quadrants (whole-pixel mode stressor).
                "quadrants" => match (x < TILE / 2, y < TILE / 2) {
                    (true, true) => [0.9, 0.05, 0.05, 1.0],
                    (false, true) => [0.05, 0.9, 0.05, 1.0],
                    (true, false) => [0.05, 0.05, 0.9, 1.0],
                    (false, false) => [0.6, 0.6, 0.0, 1.0],
                },
                // HDR super-brights at mixed alpha (§6.4).
                "hdr" => {
                    let a = if (x / 32) % 2 == 0 { 1.0 } else { 0.5 };
                    [3.5 * a, 0.0, 1.25 * a, a]
                }
                // Seeded noise: alpha and premult-valid color.
                "noise" => {
                    let a = (splitmix64(&mut rng) % 4096) as f32 / 4095.0;
                    let r = (splitmix64(&mut rng) % 4096) as f32 / 4095.0 * a;
                    let g = (splitmix64(&mut rng) % 4096) as f32 / 4095.0 * a;
                    let b = (splitmix64(&mut rng) % 4096) as f32 / 4095.0 * a;
                    [r, g, b, a]
                }
                // Alpha extremes checkerboard: exactly 0 and exactly 1.
                "alpha-extremes" => {
                    if (x / 16 + y / 16) % 2 == 0 {
                        [0.0, 0.0, 0.0, 0.0]
                    } else {
                        [0.25, 0.5, 0.75, 1.0]
                    }
                }
                // PARAMETRIC STATION source — a resolved SHAPE page
                // (§20.2): a soft-edged disc's coverage × a solid linear
                // fill, PREMULTIPLIED exactly as core's Door-1 coverage ×
                // FillConvert product (`premultiply_coverage(fill, c)`).
                // The 1-px anti-aliased annulus exercises the fractional-
                // coverage regime a filled vector edge produces.
                "shape-page" => {
                    let (cx, cy) = ((TILE as f32) / 2.0, (TILE as f32) / 2.0);
                    let dx = x as f32 + 0.5 - cx;
                    let dy = y as f32 + 0.5 - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    // radius 96, 1.5-px AA ramp inward (coverage 0→1).
                    let c = ((96.0 - dist) / 1.5).clamp(0.0, 1.0);
                    let fill = [0.82, 0.31, 0.08]; // straight linear RGB
                    [fill[0] * c, fill[1] * c, fill[2] * c, c]
                }
                // PARAMETRIC STATION source — a resolved TEXT page
                // (§23.2): near-black glyph strokes over transparency, thin
                // AA vertical + horizontal bars, PREMULTIPLIED (fill ×
                // coverage) — the Door-6 page product's fractional-edge
                // structure (stem/serif coverage the kernels must blend).
                "text-page" => {
                    let fx = (x % 16) as f32; // 3-px stems every 16 px
                    let fy = (y % 24) as f32; // 2-px bars every 24 px
                    let cx = (2.0 - (fx - 1.0).abs()).clamp(0.0, 1.0);
                    let cy = (1.5 - (fy - 0.5).abs()).clamp(0.0, 1.0);
                    let c = cx.max(cy);
                    let fill = [0.04, 0.04, 0.05]; // straight near-black ink
                    [fill[0] * c, fill[1] * c, fill[2] * c, c]
                }
                other => unreachable!("unknown pattern {other}"),
            };
            t.set(x, y, v);
        }
    }
    t
}

/// The in-crate kernel corpus: all 27 modes × (src, dst) pattern pairs.
/// 162 cases (27 × 6), each deterministic (fixed seeds).
pub fn kernel_corpus() -> Vec<KernelCase> {
    let pairs: [(&str, &str, f32); 6] = [
        ("alpha-ramp", "gradient", 1.0),
        ("gradient", "quadrants", 0.5),
        ("quadrants", "alpha-ramp", 1.0),
        ("hdr", "gradient", 1.0),
        ("noise", "noise", 0.75),
        ("alpha-extremes", "quadrants", 1.0),
    ];
    let mut out = Vec::new();
    for mode in BlendMode::ALL {
        for (i, (sp, dp, opacity)) in pairs.iter().enumerate() {
            out.push(KernelCase {
                name: format!("{mode:?}/{sp}-over-{dp}@{opacity}"),
                mode,
                src: pattern(sp, 0xC0FFEE00 + i as u64),
                dst: pattern(dp, 0xB15C0117u64 ^ ((i as u64) << 8)),
                opacity: *opacity,
                ctx: DissolveCtx {
                    bn_off: (17, 42),
                    tile_origin: (256, -256),
                },
                coverage: None,
            });
        }
    }
    out
}

/// The §34.3 PARAMETRIC STATION (composite-legs slice): parametric content
/// (shape §20.2 / text §23.2) composited through the CPU-reference-vs-GPU
/// parity oracle. The `src` tiles are resolved ComposedPage products — a
/// filled shape's coverage × fill and a text page's glyph coverage, both
/// PREMULTIPLIED exactly as core's `resolve_parametric_page` emits (Door-1
/// coverage × FillConvert / Door-6 layout — the tile-level pixels the
/// render compositor's Shape/Text arms feed into these very kernels). This
/// is the whole point of the slice: parametric content now RIDES the same
/// ≤1/255 gate raster/blend/group content always had.
///
/// The station stays in-crate and text/vector-service-free (render never
/// links appname-text/appname-vector): a page is raster-like once resolved,
/// so its parity is a property of the §4.5 kernels over premultiplied page
/// pixels — precisely what a [`KernelCase`] exercises. All 27 modes × the
/// shape/text page sources × representative backdrops, opacity 1.0 and 0.5.
pub fn parametric_station() -> Vec<KernelCase> {
    // (source page, backdrop, opacity) — the page over an opaque photo-like
    // backdrop (the canvas case) and over a fractional backdrop (a shape/
    // text layer above other content), at full and half layer opacity.
    let pairs: [(&str, &str, f32); 4] = [
        ("shape-page", "quadrants", 1.0),
        ("shape-page", "gradient", 0.5),
        ("text-page", "alpha-ramp", 1.0),
        ("text-page", "hdr", 1.0),
    ];
    let mut out = Vec::new();
    for mode in BlendMode::ALL {
        for (i, (sp, dp, opacity)) in pairs.iter().enumerate() {
            out.push(KernelCase {
                name: format!("station/{mode:?}/{sp}-over-{dp}@{opacity}"),
                mode,
                src: pattern(sp, 0x5A5E0000 + i as u64),
                dst: pattern(dp, 0x7E570000u64 ^ ((i as u64) << 8)),
                opacity: *opacity,
                ctx: DissolveCtx {
                    bn_off: (5, 23),
                    tile_origin: (-512, 512),
                },
                coverage: None,
            });
        }
    }
    out
}

/// The §34.3 §11.2 MASKED corpus (CP-0.5 mask row): the coverage-multiply
/// (`src *= E`) composited through the CPU-oracle-vs-GPU parity gate at
/// ≤1/255. Each case carries a per-pixel coverage plane E spanning the
/// regimes an unfeathered raster mask produces — E ∈ {0, 1} (hide_all /
/// reveal_all), a fractional ramp (density-scaled coverage), and a
/// soft raster edge — over the standard src/dst patterns across all 27
/// modes. Once the source is multiplied by E it is raster-like, so its
/// parity is a property of the §4.5 kernels over the E-scaled premultiplied
/// source: exactly a [`KernelCase`]. This is the whole point — masked
/// content now RIDES the same ≤1/255 gate raster content always had, and
/// the multiply is bit-equal cross-path (a single correctly-rounded op).
pub fn masked_corpus() -> Vec<KernelCase> {
    let cov = |kind: &str| -> Vec<f32> {
        let mut e = vec![0.0f32; TILE * TILE];
        for y in 0..TILE {
            for x in 0..TILE {
                e[y * TILE + x] = match kind {
                    // Hard hide/reveal checker: E is exactly 0 or 1.
                    "checker" => {
                        if (x / 16 + y / 16) % 2 == 0 {
                            0.0
                        } else {
                            1.0
                        }
                    }
                    // Fractional horizontal ramp 0→1 (density-scaled cov).
                    "ramp" => x as f32 / (TILE - 1) as f32,
                    // Uniform half coverage (a density-0.5 reveal_all).
                    "half" => 0.5,
                    // A soft raster reveal edge (unfeathered — the coverage
                    // buffer itself carries the AA ramp, no §11.2 Gaussian).
                    "soft-disc" => {
                        let (cx, cy) = (128.0f32, 128.0f32);
                        let dx = x as f32 + 0.5 - cx;
                        let dy = y as f32 + 0.5 - cy;
                        let d = (dx * dx + dy * dy).sqrt();
                        ((110.0 - d) / 6.0).clamp(0.0, 1.0)
                    }
                    _ => 1.0,
                };
            }
        }
        e
    };
    let pairs: [(&str, &str, &str, f32); 4] = [
        ("alpha-ramp", "quadrants", "checker", 1.0),
        ("gradient", "alpha-ramp", "ramp", 1.0),
        ("quadrants", "gradient", "soft-disc", 1.0),
        ("hdr", "quadrants", "half", 0.75),
    ];
    let mut out = Vec::new();
    for mode in BlendMode::ALL {
        for (i, (sp, dp, cp, opacity)) in pairs.iter().enumerate() {
            out.push(KernelCase {
                name: format!("masked/{mode:?}/{sp}x{cp}-over-{dp}@{opacity}"),
                mode,
                src: pattern(sp, 0x11A50000 + i as u64),
                dst: pattern(dp, 0x0C0F0000u64 ^ ((i as u64) << 8)),
                opacity: *opacity,
                ctx: DissolveCtx {
                    bn_off: (9, 13),
                    tile_origin: (128, -384),
                },
                coverage: Some(cov(cp)),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Self-parity: identical backends → zero diff, SSIM 1, no Dissolve
    /// mismatches — and the corpus covers all 27 modes.
    #[test]
    fn cpu_self_parity_is_exact() {
        let corpus = kernel_corpus();
        assert_eq!(corpus.len(), 27 * 6);
        let report = run_parity(&mut CpuBackend, &mut CpuBackend, &corpus);
        assert_eq!(report.max_abs_8bit, 0.0);
        assert_eq!(report.min_ssim, 1.0);
        assert_eq!(report.dissolve_coverage_mismatches, 0);
        assert_eq!(report.dissolve_determinism_failures, 0);
        report.gate().expect("self-parity gates green");
    }

    /// The detector detects: a single-pixel 2/255 error trips the
    /// max-abs gate; a 0.4/255 error passes it (the gate is a real
    /// threshold, not a rubber stamp). A Dissolve single-BIT flip trips
    /// the bit-exactness verdict.
    #[test]
    fn fault_injection_trips_the_gate() {
        struct Faulty {
            inner: CpuBackend,
            delta: f32,
        }
        impl CompositeBackend for Faulty {
            fn name(&self) -> &str {
                "faulty"
            }
            fn composite(&mut self, case: &KernelCase) -> TileRgba {
                let mut t = self.inner.composite(case);
                let mut p = t.get(128, 128);
                p[1] += self.delta;
                t.set(128, 128, p);
                t
            }
        }
        let corpus: Vec<KernelCase> = kernel_corpus()
            .into_iter()
            .filter(|c| c.mode != BlendMode::Dissolve)
            .collect();

        let mut big = Faulty {
            inner: CpuBackend,
            delta: 2.0 / 255.0,
        };
        let report = run_parity(&mut big, &mut CpuBackend, &corpus);
        assert!(
            matches!(report.gate(), Err(ParityFailure::MaxAbs { .. })),
            "{report:?}"
        );

        let mut small = Faulty {
            inner: CpuBackend,
            delta: 0.4 / 255.0,
        };
        let report = run_parity(&mut small, &mut CpuBackend, &corpus);
        assert!(report.max_abs_8bit <= 1.0 && report.max_abs_8bit > 0.0);
        report.gate().expect("sub-threshold error passes max-abs");

        // Dissolve coverage: one flipped pass/fail decision = failure.
        // (The flip shows in the transparent-backdrop coverage alpha.)
        struct CoverageFlip;
        impl CompositeBackend for CoverageFlip {
            fn name(&self) -> &str {
                "coverage-flip"
            }
            fn composite(&mut self, case: &KernelCase) -> TileRgba {
                let mut t = CpuBackend.composite(case);
                let mut p = t.get(31, 47);
                p[3] = if p[3] == 0.0 { 1.0 } else { 0.0 };
                t.set(31, 47, p);
                t
            }
        }
        let dissolve: Vec<KernelCase> = kernel_corpus()
            .into_iter()
            .filter(|c| c.mode == BlendMode::Dissolve)
            .collect();
        assert!(!dissolve.is_empty());
        let report = run_parity(&mut CoverageFlip, &mut CpuBackend, &dissolve);
        assert!(
            matches!(report.gate(), Err(ParityFailure::DissolveCoverage { .. })),
            "{report:?}"
        );

        // Dissolve own-baseline determinism: a backend that answers
        // differently on the double-run = failure.
        struct Flickery(u32);
        impl CompositeBackend for Flickery {
            fn name(&self) -> &str {
                "flickery"
            }
            fn composite(&mut self, case: &KernelCase) -> TileRgba {
                let mut t = CpuBackend.composite(case);
                self.0 += 1;
                if self.0.is_multiple_of(2) {
                    let mut p = t.get(0, 0);
                    p[0] += 1.0;
                    t.set(0, 0, p);
                }
                t
            }
        }
        let report = run_parity(&mut Flickery(0), &mut CpuBackend, &dissolve);
        assert!(
            matches!(
                report.gate(),
                Err(ParityFailure::DissolveDeterminism { .. })
            ),
            "{report:?}"
        );
    }

    /// SSIM detects structural corruption that stays under the max-abs
    /// radar in aggregate: zeroing one 8×8 window's green channel is a
    /// large local structural change — SSIM must drop below 0.999 for
    /// at least one case even though most pixels are untouched.
    #[test]
    fn ssim_catches_structural_corruption() {
        struct WindowKiller;
        impl CompositeBackend for WindowKiller {
            fn name(&self) -> &str {
                "window-killer"
            }
            fn composite(&mut self, case: &KernelCase) -> TileRgba {
                let mut t = CpuBackend.composite(case);
                for y in 64..72 {
                    for x in 64..72 {
                        let mut p = t.get(x, y);
                        p[0] = 0.0;
                        p[1] = 0.0;
                        p[2] = 0.0;
                        t.set(x, y, p);
                    }
                }
                t
            }
        }
        let corpus: Vec<KernelCase> = kernel_corpus()
            .into_iter()
            .filter(|c| c.mode == BlendMode::Normal)
            .collect();
        let report = run_parity(&mut WindowKiller, &mut CpuBackend, &corpus);
        assert!(
            report.min_ssim < 0.999,
            "structural kill must show in SSIM: {report:?}"
        );
    }

    /// The non-finite poison rule detects: a backend that leaks ONE NaN
    /// scalar must FAIL the gate — under plain f32::max folding it
    /// would gate green (IEEE maxNum ignores NaN operands).
    #[test]
    fn nan_poison_fails_the_gate() {
        struct NanLeak;
        impl CompositeBackend for NanLeak {
            fn name(&self) -> &str {
                "nan-leak"
            }
            fn composite(&mut self, case: &KernelCase) -> TileRgba {
                let mut t = CpuBackend.composite(case);
                let mut p = t.get(200, 17);
                p[2] = f32::NAN;
                t.set(200, 17, p);
                t
            }
        }
        let corpus: Vec<KernelCase> = kernel_corpus()
            .into_iter()
            .filter(|c| c.mode == BlendMode::Multiply)
            .collect();
        let report = run_parity(&mut NanLeak, &mut CpuBackend, &corpus);
        assert!(
            matches!(report.gate(), Err(ParityFailure::MaxAbs { .. })),
            "one NaN scalar must trip the gate: {report:?}"
        );
        assert_eq!(report.max_abs_8bit, f32::INFINITY);
    }

    /// §34.3 PARAMETRIC STATION: parametric page content (shape §20.2 /
    /// text §23.2) composites through the parity oracle at ≤1/255. The
    /// station covers all 27 modes; self-parity gates green (a page is
    /// raster-like — its parity is a §4.5-kernel property); the same
    /// detector that guards raster content trips on a 2/255 station error
    /// (parametric content is under the SAME gate now, not a weaker one);
    /// and every station source is premultiplied-valid (rgb ≤ a — the
    /// resolved-page invariant, no §6.4 super-bright fabricated at a
    /// coverage edge).
    #[test]
    fn parametric_station_rides_the_parity_gate() {
        let station = parametric_station();
        assert_eq!(
            station.len(),
            27 * 4,
            "all 27 modes × 4 page/backdrop pairs"
        );
        let mut modes = std::collections::BTreeSet::new();
        for case in &station {
            modes.insert(format!("{:?}", case.mode));
            for px in case.src.as_slice().chunks_exact(4) {
                let a = px[3];
                for &c in &px[..3] {
                    assert!(
                        c <= a + 1e-6 && c >= -1e-6,
                        "station page source premultiplied-valid ({c} ≤ {a}) in {}",
                        case.name
                    );
                }
            }
        }
        assert_eq!(modes.len(), 27, "the station spans every §4.5 mode");

        // Self-parity: the oracle vs itself is exact — parametric content
        // meets the ≤1/255 gate (SSIM 1, no Dissolve mismatch).
        let report = run_parity(&mut CpuBackend, &mut CpuBackend, &station);
        assert_eq!(report.max_abs_8bit, 0.0);
        report
            .gate()
            .expect("parametric station gates green on self-parity");

        // The detector is real on the station too (not a weaker gate for
        // parametric content): a 2/255 single-pixel error trips it.
        struct Faulty;
        impl CompositeBackend for Faulty {
            fn name(&self) -> &str {
                "faulty-station"
            }
            fn composite(&mut self, case: &KernelCase) -> TileRgba {
                let mut t = CpuBackend.composite(case);
                let mut p = t.get(70, 70);
                p[1] += 2.0 / 255.0;
                t.set(70, 70, p);
                t
            }
        }
        let no_dissolve: Vec<KernelCase> = parametric_station()
            .into_iter()
            .filter(|c| c.mode != BlendMode::Dissolve)
            .collect();
        let report = run_parity(&mut Faulty, &mut CpuBackend, &no_dissolve);
        assert!(
            matches!(report.gate(), Err(ParityFailure::MaxAbs { .. })),
            "the station's ≤1/255 gate is real: {report:?}"
        );
    }

    /// §11.2 MASKED corpus rides the parity gate: the coverage-multiply
    /// (`src *= E`) is bit-exact on self-parity (a single multiply — SSIM
    /// 1, no Dissolve mismatch), spans all 27 modes, its E planes are
    /// valid coverage (∈ [0, 1]) with genuine masking (some E == 0, some
    /// == 1, some fractional), and the SAME ≤1/255 detector trips on a
    /// 2/255 masked-case error (masked content is under the same gate).
    #[test]
    fn masked_corpus_rides_the_parity_gate() {
        let corpus = masked_corpus();
        assert_eq!(corpus.len(), 27 * 4, "all 27 modes × 4 mask/backdrop pairs");
        let mut modes = std::collections::BTreeSet::new();
        let (mut saw_zero, mut saw_one, mut saw_frac) = (false, false, false);
        for case in &corpus {
            modes.insert(format!("{:?}", case.mode));
            let e = case
                .coverage
                .as_ref()
                .expect("masked case carries coverage");
            assert_eq!(e.len(), TILE * TILE);
            for &v in e {
                assert!((0.0..=1.0).contains(&v), "coverage E ∈ [0,1]: {v}");
                if v == 0.0 {
                    saw_zero = true;
                } else if v == 1.0 {
                    saw_one = true;
                } else {
                    saw_frac = true;
                }
            }
        }
        assert_eq!(modes.len(), 27, "the masked corpus spans every §4.5 mode");
        assert!(
            saw_zero && saw_one && saw_frac,
            "coverage must exercise hide (0), reveal (1), and fractional"
        );

        // Self-parity: the coverage-multiply is exact against the oracle.
        let report = run_parity(&mut CpuBackend, &mut CpuBackend, &corpus);
        assert_eq!(report.max_abs_8bit, 0.0);
        assert_eq!(report.min_ssim, 1.0);
        assert_eq!(report.dissolve_coverage_mismatches, 0);
        report
            .gate()
            .expect("masked corpus gates green on self-parity");

        // The detector is real on masked content too (not a weaker gate):
        // a 2/255 single-pixel error trips the ≤1/255 gate. (128,128) is
        // fully covered in every pair (checker cell, ramp≈0.5, disc
        // center = 1, half = 0.5), so the error survives the multiply.
        struct Faulty;
        impl CompositeBackend for Faulty {
            fn name(&self) -> &str {
                "faulty-masked"
            }
            fn composite(&mut self, case: &KernelCase) -> TileRgba {
                let mut t = CpuBackend.composite(case);
                let mut p = t.get(128, 128);
                p[1] += 2.0 / 255.0;
                t.set(128, 128, p);
                t
            }
        }
        let no_dissolve: Vec<KernelCase> = masked_corpus()
            .into_iter()
            .filter(|c| c.mode != BlendMode::Dissolve)
            .collect();
        let report = run_parity(&mut Faulty, &mut CpuBackend, &no_dissolve);
        assert!(
            matches!(report.gate(), Err(ParityFailure::MaxAbs { .. })),
            "the masked ≤1/255 gate is real: {report:?}"
        );
    }

    /// Corpus determinism: two independent builds are bit-identical
    /// (the §34.5 fixture discipline — no ambient state, fixed seeds).
    #[test]
    fn corpus_is_deterministic() {
        let a = kernel_corpus();
        let b = kernel_corpus();
        assert_eq!(a.len(), b.len());
        for (ca, cb) in a.iter().zip(&b) {
            assert_eq!(ca.name, cb.name);
            assert!(
                ca.src == cb.src && ca.dst == cb.dst,
                "{}: fixture drift",
                ca.name
            );
        }
    }
}
