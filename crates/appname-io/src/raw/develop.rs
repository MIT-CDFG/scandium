// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The §8.3 develop pipeline, stages 2–6, as PURE PLANAR FUNCTIONS over a
//! decoded CFA-plane abstraction — Appname's OWN implementations per Part
//! 43 §43.4 stage ownership ("Stages 2–6 are Appname's own SIMD
//! implementations in `appname-io` of the named public algorithms").
//!
//! Spec: ES §8.3 stage formulas [NORMATIVE]; Part 43 §43.3 pipeline order
//! (fixed forever, reserved sub-stage slots 4a/4b/6a/6b), §43.4
//! determinism rules; dossier `spec/algorithms/dossiers/raw-demosaic.yaml`
//! (AHD election — Hirakawa–Parks 2005, `ahd05`; own Rust from the paper;
//! bilinear draft/preview path; X-Trans via the LibRaw dependency path).
//!
//! **Determinism (§43.4).** Every function here is a pure function of its
//! arguments: no clocks, no RNG, no shared accumulators. The one global
//! reduction — `Auto` white balance's gray-world sums — computes per-band
//! partial sums in `f64` over fixed [`BAND_ROWS`]-row bands and combines
//! them in ascending band index order (§43.4 rule 2), so gains are
//! identical for any future worker count. Per-pixel math is `f32` in the
//! documented §8.3 operation order; no fast-math is enabled anywhere in
//! this crate (rule 3).
//!
//! **Stage 1 lives in [`crate::raw::stage1`]** (the sanitized LibRaw
//! wrapper over the `sys/libraw-sys` facade — record consumed this
//! slice). Everything below starts from the stage-1 outputs a decoder
//! hands over (CFA plane + levels + pattern + multipliers + matrix),
//! which is also exactly what the §43.7 T-01 unit vectors feed; the
//! mosaic hop itself is behind the recorded egress gap (stage1 doc).
//!
//! **Stage 6a note (the "tone curve" that is NOT here):** §43.3 reserves
//! contrast/shadows tone at slot 6a for Post-1.0 and pins that the v1
//! surface exposes exactly the five v1 rows — "Contrast, shadows, … are
//! explicitly not v1 features … answered by the Part 38 parity matrix,
//! not by ad-hoc additions." The develop result stays LINEAR ("no tone
//! curve baked", §8.3 result contract). The slot exists in the pipeline
//! order; its math deliberately does not.

use crate::error::CodecError;
use crate::raw::params::{Demosaic, RawDevelopParams, WhiteBalance};

/// §43.4 rule 2: fixed band height for the deterministic banded
/// reductions (and, later, the thread partition).
pub const BAND_ROWS: usize = 256;

/// Normalized sensor value at/above which a photosite counts as clipped
/// (stage-2 output is scaled so saturation = 1.0).
const CLIP_T: f32 = 0.995;

/// Bayer CFA layout (2×2 tile). X-Trans/Foveon are NOT represented here:
/// non-Bayer sensors take LibRaw's sensor-appropriate path per §8.3 stage
/// 4 / the dossier io_contract — typed-NI at [`crate::raw::stage1`]'s
/// classifier until the recorded egress gap resolves (stage1 doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfaPattern {
    Rggb,
    Bggr,
    Grbg,
    Gbrg,
}

impl CfaPattern {
    /// Channel (0 = R, 1 = G, 2 = B) of the photosite at `(x, y)`.
    #[inline]
    pub fn color_at(self, x: usize, y: usize) -> usize {
        // 2×2 tile as [row0col0, row0col1, row1col0, row1col1].
        let tile: [usize; 4] = match self {
            CfaPattern::Rggb => [0, 1, 1, 2],
            CfaPattern::Bggr => [2, 1, 1, 0],
            CfaPattern::Grbg => [1, 0, 2, 1],
            CfaPattern::Gbrg => [1, 2, 0, 1],
        };
        tile[(y & 1) * 2 + (x & 1)]
    }
}

/// The stage-2 output: one normalized channel per photosite (§43.4 "the
/// normalized CFA plane"), row-major.
#[derive(Debug, Clone)]
pub struct CfaPlane {
    pub width: usize,
    pub height: usize,
    pub pattern: CfaPattern,
    pub data: Vec<f32>,
}

/// Stage-6 camera-RGB → XYZ(D50) matrix resolution source, recorded as
/// `raw.develop.profile.matrixSource` (§43.2 precedence order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixSource {
    /// §43.2 rule 1 — the matrix carried in the file itself.
    Embedded,
    /// §43.2 rule 2 — LibRaw's shipped public per-camera table
    /// (record-gated with the binding).
    LibRawTable,
    /// §43.2 rule 3 — neither available: sRGB primaries assumed, an
    /// `Approximated` report item (`raw.camera_matrix`, RAW-02) recorded.
    AssumedSrgb,
}

/// XYZ(D65) → linear sRGB (IEC 61966-2-1 primaries) — the §43.2 rule-3
/// assumption matrix, and the fixed metric transform inside AHD's
/// homogeneity maps (metric-only there: it never touches output values).
///
/// **ADVISORY GUARD (bound to this slice, INTEGRATION-log 2026-08-17):
/// this constant is D65-NATIVE and MUST NOT be used for ICC colorants.**
/// §43.2's generated-profile table pins the ICC connection space at D50
/// (`wtpt` = D50; `rXYZ/gXYZ/bXYZ` = columns of the camera-RGB →
/// XYZ(**D50**) matrix). Stage-7 profile generation goes through
/// [`stage7_colorants_d50`] (Bradford-adapted), never through this
/// constant or [`SRGB_TO_XYZ`] directly.
const XYZ_TO_SRGB: [[f32; 3]; 3] = [
    [3.240_454_2, -1.537_138_5, -0.498_531_4],
    [-0.969_266, 1.876_010_8, 0.041_556_0],
    [0.055_643_4, -0.204_025_9, 1.057_225_2],
];

/// Linear sRGB → XYZ(D65) (inverse of [`XYZ_TO_SRGB`], fixed constants).
///
/// Same advisory guard as [`XYZ_TO_SRGB`]: D65-native, metric/assumption
/// use only — ICC colorants come from [`stage7_colorants_d50`].
const SRGB_TO_XYZ: [[f32; 3]; 3] = [
    [0.412_456_4, 0.357_576_1, 0.180_437_5],
    [0.212_672_9, 0.715_152_2, 0.072_175_0],
    [0.019_333_9, 0.119_192, 0.950_304_1],
];

// ---------------------------------------------------------------------------
// Stage-7 colorant source (ADVISORY 1: D65 metric constant vs D50 ICC
// convention — the bound advisory's fix)
// ---------------------------------------------------------------------------

/// Bradford cone-response matrix (the ICC-recommended chromatic
/// adaptation transform; published constants).
const BRADFORD: [[f64; 3]; 3] = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];

/// Bradford inverse (published constants).
const BRADFORD_INV: [[f64; 3]; 3] = [
    [0.986_992_9, -0.147_054_3, 0.159_962_7],
    [0.432_305_3, 0.518_360_2, 0.049_291_2],
    [-0.008_528_7, 0.040_042_8, 0.968_486_7],
];

/// D65 white (ASTM E308-01 2°, the sRGB reference white).
const WHITE_D65: [f64; 3] = [0.95047, 1.0, 1.08883];

/// D50 white — the ICC profile connection space white (§5.5 / §43.2
/// `wtpt` row; matches the ICC s15Fixed16 values icc_synth pins).
const WHITE_D50: [f64; 3] = [0.9642, 1.0, 0.8249];

fn mat_mul_f64(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0f64; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

/// The Bradford D65→D50 adaptation matrix, computed in `f64` from the
/// pinned published constants (deterministic — pure constant folding).
fn bradford_d65_to_d50() -> [[f64; 3]; 3] {
    let src = [
        BRADFORD[0][0] * WHITE_D65[0]
            + BRADFORD[0][1] * WHITE_D65[1]
            + BRADFORD[0][2] * WHITE_D65[2],
        BRADFORD[1][0] * WHITE_D65[0]
            + BRADFORD[1][1] * WHITE_D65[1]
            + BRADFORD[1][2] * WHITE_D65[2],
        BRADFORD[2][0] * WHITE_D65[0]
            + BRADFORD[2][1] * WHITE_D65[1]
            + BRADFORD[2][2] * WHITE_D65[2],
    ];
    let dst = [
        BRADFORD[0][0] * WHITE_D50[0]
            + BRADFORD[0][1] * WHITE_D50[1]
            + BRADFORD[0][2] * WHITE_D50[2],
        BRADFORD[1][0] * WHITE_D50[0]
            + BRADFORD[1][1] * WHITE_D50[1]
            + BRADFORD[1][2] * WHITE_D50[2],
        BRADFORD[2][0] * WHITE_D50[0]
            + BRADFORD[2][1] * WHITE_D50[1]
            + BRADFORD[2][2] * WHITE_D50[2],
    ];
    let scale = [
        [dst[0] / src[0], 0.0, 0.0],
        [0.0, dst[1] / src[1], 0.0],
        [0.0, 0.0, dst[2] / src[2]],
    ];
    mat_mul_f64(&BRADFORD_INV, &mat_mul_f64(&scale, &BRADFORD))
}

/// **The stage-7 ICC colorant source** (§43.2 profile table): adapt a
/// camera-RGB → XYZ(**D65**) matrix to the ICC D50 connection space via
/// Bradford. The columns of the result are the generated profile's
/// `rXYZ`/`gXYZ`/`bXYZ` values.
///
/// This function exists because of the bound advisory (INTEGRATION-log
/// 2026-08-17): [`XYZ_TO_SRGB`]/[`SRGB_TO_XYZ`] are D65-native metric
/// constants and reusing them for colorants would ship a profile whose
/// white point contradicts its primaries. Every future stage-7 emitter
/// consumes THIS function; the full profile generation (ICC byte
/// assembly + document profile-store registration + `camera/{make}/
/// {model}` naming) stays a recorded deferral until the O-COLOR profile
/// store surface lands.
pub fn stage7_colorants_d50(cam_to_xyz_d65: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let adapt = bradford_d65_to_d50();
    let m64 = [
        [
            f64::from(cam_to_xyz_d65[0][0]),
            f64::from(cam_to_xyz_d65[0][1]),
            f64::from(cam_to_xyz_d65[0][2]),
        ],
        [
            f64::from(cam_to_xyz_d65[1][0]),
            f64::from(cam_to_xyz_d65[1][1]),
            f64::from(cam_to_xyz_d65[1][2]),
        ],
        [
            f64::from(cam_to_xyz_d65[2][0]),
            f64::from(cam_to_xyz_d65[2][1]),
            f64::from(cam_to_xyz_d65[2][2]),
        ],
    ];
    let out = mat_mul_f64(&adapt, &m64);
    let mut r = [[0.0f32; 3]; 3];
    for (i, row) in r.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = out[i][j] as f32;
        }
    }
    r
}

// ---------------------------------------------------------------------------
// Stage 2 — black subtraction & scale
// ---------------------------------------------------------------------------

/// Sanitize per-CFA-position black/white levels BEFORE any stage-2
/// arithmetic — **the bound degenerate-levels advisory** (INTEGRATION-log
/// 2026-08-17): a file whose metadata yields `white == black` (or
/// non-finite levels) used to propagate non-finite values through the
/// normalize division and reach a `median9` panic inside AHD — a
/// codec-boundary panic on hostile input. Every level set entering the
/// develop pipeline passes through this check (it is called by
/// [`stage2_normalize`] itself, so the guard cannot be bypassed; the
/// stage-1 wrapper additionally calls it for early, well-named errors).
pub fn sanitize_levels(black: [f32; 4], white: [f32; 4]) -> Result<(), CodecError> {
    for pos in 0..4 {
        let (b, w) = (black[pos], white[pos]);
        if !b.is_finite() || !w.is_finite() {
            return Err(CodecError::corrupt(format!(
                "raw.levels_degenerate: non-finite black/white ({b}/{w}) at CFA position {pos}"
            )));
        }
        if w <= b {
            return Err(CodecError::corrupt(format!(
                "raw.levels_degenerate: white {w} <= black {b} at CFA position {pos}"
            )));
        }
    }
    Ok(())
}

/// §8.3 stage 2 [NORMATIVE]: per channel `v' = (v − black) / (white −
/// black)`, clamped at 0. `black`/`white` are per CFA-tile position
/// (`(y & 1) * 2 + (x & 1)` — LibRaw's `cblack` shape), so the two greens
/// may carry distinct levels.
///
/// Degenerate levels (white ≤ black, non-finite) are a typed
/// `error.codec.corrupt` refusal, never a downstream panic — the bound
/// advisory ([`sanitize_levels`]).
pub fn stage2_normalize(
    raw: &[u16],
    width: usize,
    height: usize,
    pattern: CfaPattern,
    black: [f32; 4],
    white: [f32; 4],
) -> Result<CfaPlane, CodecError> {
    assert_eq!(raw.len(), width * height, "photosite count mismatch");
    sanitize_levels(black, white)?;
    let mut data = vec![0.0f32; raw.len()];
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            let pos = (y & 1) * 2 + (x & 1);
            let denom = white[pos] - black[pos];
            let v = (f32::from(raw[i]) - black[pos]) / denom;
            data[i] = v.max(0.0);
        }
    }
    Ok(CfaPlane {
        width,
        height,
        pattern,
        data,
    })
}

// ---------------------------------------------------------------------------
// Stage 3 — white balance
// ---------------------------------------------------------------------------

/// Camera as-shot multipliers `(r, g1, b, g2)` → `[r, g, b]` gains
/// normalized so green = 1 (green = mean of the two green multipliers
/// when both are non-zero; LibRaw files with `g2 = 0` mean "same as g1").
pub fn as_shot_gains(mul: [f32; 4]) -> [f32; 3] {
    let g = if mul[3] > 0.0 {
        0.5 * (mul[1] + mul[3])
    } else {
        mul[1]
    };
    [mul[0] / g, 1.0, mul[2] / g]
}

/// §8.3 `Auto`: gray-world gains over non-clipped photosites,
/// center-weighted. Deterministic per §43.4 rule 2: per-band `f64`
/// partial sums over fixed [`BAND_ROWS`]-row bands, combined in ascending
/// band order.
///
/// Recorded election (lane log): the center weight is the separable tent
/// `w(x, y) = (1 − |2x/(W−1) − 1|) · (1 − |2y/(H−1) − 1|)` (peak 1 at the
/// image center, 0 at the borders), computed in `f64` from integer
/// coordinates — §8.3 names center-weighting without pinning a kernel, so
/// the kernel is pinned HERE and by the goldens.
pub fn auto_gains(cfa: &CfaPlane) -> [f32; 3] {
    let (w, h) = (cfa.width, cfa.height);
    let bands = h.div_ceil(BAND_ROWS);
    // Per-band partials: weighted sum + weight per channel.
    let mut partials = vec![[[0.0f64; 2]; 3]; bands];
    for (b, part) in partials.iter_mut().enumerate() {
        let y0 = b * BAND_ROWS;
        let y1 = (y0 + BAND_ROWS).min(h);
        for y in y0..y1 {
            let wy = if h > 1 {
                1.0 - ((2.0 * y as f64) / ((h - 1) as f64) - 1.0).abs()
            } else {
                1.0
            };
            for x in 0..w {
                let v = cfa.data[y * w + x];
                if v >= CLIP_T {
                    continue; // §8.3: excluding clipped pixels
                }
                let wx = if w > 1 {
                    1.0 - ((2.0 * x as f64) / ((w - 1) as f64) - 1.0).abs()
                } else {
                    1.0
                };
                let c = cfa.pattern.color_at(x, y);
                part[c][0] += f64::from(v) * wx * wy;
                part[c][1] += wx * wy;
            }
        }
    }
    // Ascending band-index merge (§43.4 rule 2).
    let mut acc = [[0.0f64; 2]; 3];
    for part in &partials {
        for c in 0..3 {
            acc[c][0] += part[c][0];
            acc[c][1] += part[c][1];
        }
    }
    let mean = |c: usize| -> f64 {
        if acc[c][1] > 0.0 {
            acc[c][0] / acc[c][1]
        } else {
            1.0
        }
    };
    let (r, g, b) = (mean(0), mean(1), mean(2));
    // Gray world: scale every channel's mean to green's.
    let safe = |m: f64| if m > 0.0 { m } else { 1.0 };
    [(g / safe(r)) as f32, 1.0, (g / safe(b)) as f32]
}

/// §8.3 `Custom`: `(temp_k, tint)` through the standard CCT→xy relation,
/// then to camera-space gains via the inverse camera matrix
/// (`cam_from_xyz` = the inverse of the resolved stage-6 matrix; under
/// the §43.2 rule-3 assumption pass [`SRGB_TO_XYZ`]'s inverse, i.e.
/// [`xyz_to_assumed_cam`]).
///
/// Recorded elections (lane log): CCT→xy uses the CIE daylight-locus
/// polynomial for `T ≥ 4000 K` and the Kim et al. cubic Planckian
/// approximation below; tint is a Δuv offset perpendicular to the locus
/// in CIE 1960 uv, `Δuv = tint · 3.0e-4` (positive toward magenta), the
/// locus tangent taken by central difference at ±100 K.
pub fn custom_gains(temp_k: f32, tint: f32, cam_from_xyz: &[[f32; 3]; 3]) -> [f32; 3] {
    let xy = cct_to_xy(f64::from(temp_k));
    // Tangent of the locus for the perpendicular tint axis.
    let a = cct_to_xy(f64::from(temp_k) - 100.0);
    let b = cct_to_xy(f64::from(temp_k) + 100.0);
    let (u, v) = xy_to_uv(xy);
    let (ua, va) = xy_to_uv(a);
    let (ub, vb) = xy_to_uv(b);
    let (mut tu, mut tv) = (ub - ua, vb - va);
    let len = (tu * tu + tv * tv).sqrt();
    if len > 0.0 {
        tu /= len;
        tv /= len;
    }
    // Perpendicular (rotate tangent by -90°); positive tint => toward
    // magenta (below the locus in uv for the daylight range).
    let duv = f64::from(tint) * 3.0e-4;
    let (un, vn) = (u + tv * duv, v - tu * duv);
    let (x, y) = uv_to_xy((un, vn));
    // xy → XYZ with Y = 1 (illuminant white), → camera RGB.
    let xyz = [(x / y) as f32, 1.0, ((1.0 - x - y) / y) as f32];
    let cam = mat_mul_vec(cam_from_xyz, xyz);
    let g = if cam[1] != 0.0 { cam[1] } else { 1.0 };
    [g / cam[0].max(1e-6), 1.0, g / cam[2].max(1e-6)]
}

/// The §43.2 rule-3 `cam_from_xyz` (camera treated as linear sRGB).
pub fn xyz_to_assumed_cam() -> [[f32; 3]; 3] {
    XYZ_TO_SRGB
}

fn cct_to_xy(t: f64) -> (f64, f64) {
    let t = t.clamp(1667.0, 50_000.0);
    let x = if t < 4000.0 {
        // Kim et al. (2002) cubic Planckian-locus approximation (single x
        // polynomial over 1667–4000 K).
        let (r1, r2, r3) = (1e3 / t, 1e6 / (t * t), 1e9 / (t * t * t));
        -0.2661239 * r3 - 0.2343589 * r2 + 0.8776956 * r1 + 0.179910
    } else {
        // CIE daylight locus (4000–25000 K arms; ≥ 25000 K extrapolates
        // the upper arm — documented election, in-range for §43.3's cap).
        let (r1, r2, r3) = (1e3 / t, 1e6 / (t * t), 1e9 / (t * t * t));
        if t <= 7000.0 {
            0.244063 + 0.09911 * r1 + 2.9678 * r2 - 4.607 * r3
        } else {
            0.237040 + 0.24748 * r1 + 1.9018 * r2 - 2.0064 * r3
        }
    };
    let y = if t < 4000.0 {
        // Kim et al. companion polynomial for the Planckian arm.
        let x2 = x * x;
        let x3 = x2 * x;
        if t < 2222.0 {
            -1.1063814 * x3 - 1.34811020 * x2 + 2.18555832 * x - 0.20219683
        } else {
            -0.9549476 * x3 - 1.37418593 * x2 + 2.09137015 * x - 0.16748867
        }
    } else {
        // CIE daylight companion: y = -3x² + 2.87x - 0.275.
        -3.000 * x * x + 2.870 * x - 0.275
    };
    (x, y)
}

fn xy_to_uv((x, y): (f64, f64)) -> (f64, f64) {
    let d = -2.0 * x + 12.0 * y + 3.0;
    (4.0 * x / d, 6.0 * y / d)
}

fn uv_to_xy((u, v): (f64, f64)) -> (f64, f64) {
    let d = 2.0 * u - 8.0 * v + 4.0;
    (3.0 * u / d, 2.0 * v / d)
}

fn mat_mul_vec(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Stage 3 [NORMATIVE]: multiply CFA channels by gains BEFORE demosaic
/// (§8.3).
pub fn apply_wb(cfa: &mut CfaPlane, gains: [f32; 3]) {
    let w = cfa.width;
    for y in 0..cfa.height {
        for x in 0..w {
            cfa.data[y * w + x] *= gains[cfa.pattern.color_at(x, y)];
        }
    }
}

// ---------------------------------------------------------------------------
// Stage 4 — demosaic
// ---------------------------------------------------------------------------

/// Reflect-101 index (mirror without repeating the edge sample) — the
/// fixed, documented border rule for every neighborhood below.
#[inline]
fn refl(i: isize, n: usize) -> usize {
    let n = n as isize;
    let mut i = i;
    if n == 1 {
        return 0;
    }
    while i < 0 || i >= n {
        if i < 0 {
            i = -i;
        }
        if i >= n {
            i = 2 * (n - 1) - i;
        }
    }
    i as usize
}

/// Stage 4 dispatch (§43.3 selection table). `Vng`/`Ppg` are typed-NI in
/// this slice (user-choice algorithms; the math lands in a follow-up O-IO
/// slice — recorded deferral, lane log); `Linear`/`Ahd` (the preview path
/// and THE commit default) are implemented below.
pub fn demosaic(cfa: &CfaPlane, algo: Demosaic) -> Result<Vec<[f32; 3]>, CodecError> {
    match algo {
        Demosaic::Linear => Ok(demosaic_bilinear(cfa)),
        Demosaic::Ahd => Ok(demosaic_ahd(cfa)),
        Demosaic::Vng => Err(CodecError::UnsupportedFeature {
            feature: "raw.demosaic_vng_pending_slice".into(),
        }),
        Demosaic::Ppg => Err(CodecError::UnsupportedFeature {
            feature: "raw.demosaic_ppg_pending_slice".into(),
        }),
    }
}

/// Bilinear CFA interpolation (§43.3 `linear` — fastest/softest; the
/// dialog-preview and LOD > 0 proxy path). Each missing channel is the
/// mean of the like-colored photosites in the 3×3 neighborhood.
pub fn demosaic_bilinear(cfa: &CfaPlane) -> Vec<[f32; 3]> {
    let (w, h) = (cfa.width, cfa.height);
    let mut out = vec![[0.0f32; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0.0f32; 3];
            let mut cnt = [0u32; 3];
            for dy in -1isize..=1 {
                for dx in -1isize..=1 {
                    let (sx, sy) = (refl(x as isize + dx, w), refl(y as isize + dy, h));
                    // Channel identity follows the LOGICAL coordinate so
                    // reflected border samples keep their CFA color.
                    let c = cfa.pattern.color_at(sx, sy);
                    sum[c] += cfa.data[sy * w + sx];
                    cnt[c] += 1;
                }
            }
            let here = cfa.pattern.color_at(x, y);
            let px = &mut out[y * w + x];
            for c in 0..3 {
                px[c] = if c == here {
                    cfa.data[y * w + x]
                } else if cnt[c] > 0 {
                    sum[c] / cnt[c] as f32
                } else {
                    0.0
                };
            }
        }
    }
    out
}

/// AHD — adaptive homogeneity-directed demosaic (Hirakawa–Parks 2005,
/// dossier `raw-demosaic` math row): directional H/V interpolations,
/// homogeneity maps in a CIELab-based metric, per-pixel direction by the
/// larger 3×3-summed homogeneity (ties average both candidates), then a
/// 3×3 median chroma refine ×2. The Lab conversion is METRIC-ONLY (fixed
/// [`SRGB_TO_XYZ`] + D65 white, per the dossier's pre-color-matrix
/// contract) — it selects between candidates and never alters values.
pub fn demosaic_ahd(cfa: &CfaPlane) -> Vec<[f32; 3]> {
    let (w, h) = (cfa.width, cfa.height);
    let n = w * h;
    let at = |x: isize, y: isize| -> f32 { cfa.data[refl(y, h) * w + refl(x, w)] };
    let color = |x: usize, y: usize| cfa.pattern.color_at(x, y);

    // 1. Directional green candidates (paper eq. 4 five-tap filters).
    let mut g_h = vec![0.0f32; n];
    let mut g_v = vec![0.0f32; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let v = cfa.data[i];
            if color(x, y) == 1 {
                g_h[i] = v;
                g_v[i] = v;
            } else {
                let (xi, yi) = (x as isize, y as isize);
                g_h[i] = 0.5 * (at(xi - 1, yi) + at(xi + 1, yi))
                    + 0.25 * (2.0 * v - at(xi - 2, yi) - at(xi + 2, yi));
                g_v[i] = 0.5 * (at(xi, yi - 1) + at(xi, yi + 1))
                    + 0.25 * (2.0 * v - at(xi, yi - 2) - at(xi, yi + 2));
            }
        }
    }

    // 2. Full RGB per direction: R/B via color-difference interpolation
    //    over like-colored 3×3 neighbors (constant-chroma assumption).
    let fill = |g: &[f32]| -> Vec<[f32; 3]> {
        let mut img = vec![[0.0f32; 3]; n];
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                img[i][1] = g[i];
                let here = color(x, y);
                if here != 1 {
                    img[i][here] = cfa.data[i];
                }
                for c in [0usize, 2] {
                    if c == here {
                        continue;
                    }
                    let mut sum = 0.0f32;
                    let mut cnt = 0u32;
                    for dy in -1isize..=1 {
                        for dx in -1isize..=1 {
                            let (sx, sy) = (refl(x as isize + dx, w), refl(y as isize + dy, h));
                            if color(sx, sy) == c {
                                sum += cfa.data[sy * w + sx] - g[sy * w + sx];
                                cnt += 1;
                            }
                        }
                    }
                    img[i][c] = g[i] + if cnt > 0 { sum / cnt as f32 } else { 0.0 };
                }
            }
        }
        img
    };
    let rgb_h = fill(&g_h);
    let rgb_v = fill(&g_v);

    // 3. Metric Lab planes for both candidates.
    let lab = |img: &[[f32; 3]]| -> Vec<[f32; 3]> {
        img.iter().map(|px| rgb_to_lab_metric(*px)).collect()
    };
    let lab_h = lab(&rgb_h);
    let lab_v = lab(&rgb_v);

    // 4. Homogeneity maps (paper §III-B, radius-1 four-neighborhood with
    //    adaptive thresholds εL/εC from the direction-aligned diffs).
    let mut hom_h = vec![0u8; n];
    let mut hom_v = vec![0u8; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let nb = |lab: &[[f32; 3]], dx: isize, dy: isize| -> [f32; 3] {
                lab[refl(y as isize + dy, h) * w + refl(x as isize + dx, w)]
            };
            let ld = |a: [f32; 3], b: [f32; 3]| (a[0] - b[0]).abs();
            let cd = |a: [f32; 3], b: [f32; 3]| {
                let (da, db) = (a[1] - b[1], a[2] - b[2]);
                da * da + db * db
            };
            // Four neighbors: 0 = left, 1 = right, 2 = up, 3 = down.
            let dirs: [(isize, isize); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
            let mut lh = [0.0f32; 4];
            let mut lv = [0.0f32; 4];
            let mut ch = [0.0f32; 4];
            let mut cv = [0.0f32; 4];
            for (k, (dx, dy)) in dirs.iter().enumerate() {
                lh[k] = ld(lab_h[i], nb(&lab_h, *dx, *dy));
                lv[k] = ld(lab_v[i], nb(&lab_v, *dx, *dy));
                ch[k] = cd(lab_h[i], nb(&lab_h, *dx, *dy));
                cv[k] = cd(lab_v[i], nb(&lab_v, *dx, *dy));
            }
            let eps_l = f32::min(f32::max(lh[0], lh[1]), f32::max(lv[2], lv[3]));
            let eps_c = f32::min(f32::max(ch[0], ch[1]), f32::max(cv[2], cv[3]));
            for k in 0..4 {
                if lh[k] <= eps_l && ch[k] <= eps_c {
                    hom_h[i] += 1;
                }
                if lv[k] <= eps_l && cv[k] <= eps_c {
                    hom_v[i] += 1;
                }
            }
        }
    }

    // 5. Direction vote: 3×3-summed homogeneity, larger wins, ties
    //    average both candidates.
    let mut out = vec![[0.0f32; 3]; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut sh = 0u32;
            let mut sv = 0u32;
            for dy in -1isize..=1 {
                for dx in -1isize..=1 {
                    let j = refl(y as isize + dy, h) * w + refl(x as isize + dx, w);
                    sh += u32::from(hom_h[j]);
                    sv += u32::from(hom_v[j]);
                }
            }
            out[i] = if sh > sv {
                rgb_h[i]
            } else if sv > sh {
                rgb_v[i]
            } else {
                [
                    0.5 * (rgb_h[i][0] + rgb_v[i][0]),
                    0.5 * (rgb_h[i][1] + rgb_v[i][1]),
                    0.5 * (rgb_h[i][2] + rgb_v[i][2]),
                ]
            };
        }
    }

    // 6. 3×3 median chroma refine ×2 on (R−G) and (B−G) (dossier math
    //    row), reconstructing R/B from the filtered differences.
    for _ in 0..2 {
        let snapshot = out.clone();
        for y in 0..h {
            for x in 0..w {
                let mut dr = [0.0f32; 9];
                let mut db = [0.0f32; 9];
                let mut k = 0;
                for dy in -1isize..=1 {
                    for dx in -1isize..=1 {
                        let px = snapshot[refl(y as isize + dy, h) * w + refl(x as isize + dx, w)];
                        dr[k] = px[0] - px[1];
                        db[k] = px[2] - px[1];
                        k += 1;
                    }
                }
                let i = y * w + x;
                out[i][0] = out[i][1] + median9(&mut dr);
                out[i][2] = out[i][1] + median9(&mut db);
            }
        }
    }
    out
}

fn median9(v: &mut [f32; 9]) -> f32 {
    // total_cmp: panic-free by construction. Non-finite values cannot
    // reach here anymore (sanitize_levels, the bound advisory), but a
    // sort comparator at a codec-adjacent boundary must never be able
    // to panic regardless (defense in depth).
    v.sort_by(f32::total_cmp);
    v[4]
}

/// Metric-only Lab (fixed sRGB→XYZ + D65 white; see [`demosaic_ahd`]).
fn rgb_to_lab_metric(px: [f32; 3]) -> [f32; 3] {
    let xyz = mat_mul_vec(&SRGB_TO_XYZ, px);
    const WHITE: [f32; 3] = [0.950_47, 1.0, 1.088_83];
    let f = |t: f32| -> f32 {
        const D: f32 = 6.0 / 29.0;
        if t > D * D * D {
            t.cbrt()
        } else {
            t / (3.0 * D * D) + 4.0 / 29.0
        }
    };
    let (fx, fy, fz) = (
        f(xyz[0] / WHITE[0]),
        f(xyz[1] / WHITE[1]),
        f(xyz[2] / WHITE[2]),
    );
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

// ---------------------------------------------------------------------------
// Stage 4b — lens correction (machinery live; v1 PARAM surface stays
// reserved per §43.3)
// ---------------------------------------------------------------------------

/// A stage-4b lens-correction request: the resolved per-shot calibration
/// (from the Row-C lensdb reader through the [`crate::raw::lensdb_seam`]
/// consumer seam) plus the three §43.3 `lensCorrection` toggles — the
/// reserved schema's exact shape
/// (`{"distortion":bool,"chromaticAberration":bool,"vignette":bool}`).
///
/// **Contract position (read §43.3 to the letter):** `lensCorrection` is
/// a RESERVED Post-1.0 parameter — the v1 dialog never shows it, and the
/// v1 params layer keeps rejecting any non-null `lensCorrection` key
/// ([`crate::raw::params`], unchanged). This machinery landed at CP-1.0
/// because its former activation trigger FIRED (Row C READY + the
/// data-package record `deps/lensfun-db.yaml`, both merged; all four
/// reader inferences O-QA-RATIFIED 2026-08-17): the ENGINE leg exists,
/// callable by the Post-1.0 smart-RAW layer and by tests, while the v1
/// user surface remains exactly the five §43.3 rows. Nothing in the v1
/// codec path can construct a non-neutral request.
#[derive(Debug, Clone, Copy)]
pub struct LensCorrection<'a> {
    /// Per-shot calibration resolved via the consumer seam.
    pub calibration: &'a appname_algo::lensdb::ResolvedCalibration,
    /// Correct geometric distortion (§43.3 `distortion`).
    pub distortion: bool,
    /// Correct lateral chromatic aberration (`chromaticAberration`).
    pub chromatic_aberration: bool,
    /// Correct vignetting (`vignette`).
    pub vignette: bool,
}

/// Stage 4b (§43.3 slot between demosaic and highlight recovery —
/// "adding a parameter later never reorders existing math"): apply the
/// resolved calibration to the demosaiced image.
///
/// Documented conventions (pinned here + by tests; they ride the same
/// ratification/bounce path as the reader's four ratified inferences —
/// no lensfun-consuming golden freezes outside O-QA):
///
/// * **Radius normalization:** `r = 1` at half the smaller image
///   dimension — the published PanoTools anchor the ratified
///   `ptlens`/`poly3` convention is defined against (O-QA verdict 2:
///   "the size anchor held only possible at the radius r_src = 1.0").
///   Center and radius derive from integer pixel coordinates:
///   center `((w−1)/2, (h−1)/2)`, unit `(min(w,h)−1)/2`, all `f64`.
/// * **Direction:** the DB models map undistorted → distorted
///   (`r_d = r_u · scale(r_u)`, the reader's documented multiplier
///   convention), so correction resamples DIRECTLY: the output pixel at
///   undistorted radius `r_u` reads the source at `r_u · scale(r_u)` —
///   no polynomial inversion.
/// * **TCA composition:** per-channel scales apply to the
///   geometry-corrected source radius: `r_src(G) = r_u · s_geom`,
///   `r_src(R/B) = r_src(G) · scale_red/blue(r_src(G))`; green is the
///   TCA reference channel.
/// * **Vignetting:** divides by `falloff(r_src(G))` — calibrated
///   against source (sensor) geometry. The divisor is clamped to
///   ≥ 1e-6 so hostile user-dir coefficients can never produce
///   non-finite output (degenerate-levels advisory discipline).
/// * **Sampling:** bilinear, edge-clamped; out-of-frame sources clamp
///   to the border. No automatic crop/rescale — frame policy is the
///   Post-1.0 dialog's, not baked here.
pub fn apply_lens_correction(
    img: &[[f32; 3]],
    width: usize,
    height: usize,
    req: &LensCorrection<'_>,
) -> Vec<[f32; 3]> {
    let cal = req.calibration;
    let (w, h) = (width, height);
    let cx = (w as f64 - 1.0) / 2.0;
    let cy = (h as f64 - 1.0) / 2.0;
    let unit = ((w.min(h) as f64) - 1.0).max(1.0) / 2.0;

    // Bilinear sample with edge clamp, per channel.
    let sample = |x: f64, y: f64, c: usize| -> f32 {
        let xc = x.clamp(0.0, (w - 1) as f64);
        let yc = y.clamp(0.0, (h - 1) as f64);
        let (x0, y0) = (xc.floor() as usize, yc.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (fx, fy) = ((xc - x0 as f64) as f32, (yc - y0 as f64) as f32);
        let v00 = img[y0 * w + x0][c];
        let v10 = img[y0 * w + x1][c];
        let v01 = img[y1 * w + x0][c];
        let v11 = img[y1 * w + x1][c];
        let top = v00 + (v10 - v00) * fx;
        let bot = v01 + (v11 - v01) * fx;
        top + (bot - top) * fy
    };

    let mut out = vec![[0.0f32; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f64 - cx) / unit;
            let dy = (y as f64 - cy) / unit;
            let r_u = (dx * dx + dy * dy).sqrt();
            // Geometry (distortion) scale at the undistorted radius.
            let s_geom = match (&cal.distortion, req.distortion) {
                (Some(model), true) => sanitize_scale(model.radial_scale(r_u)),
                _ => 1.0,
            };
            let r_g = r_u * s_geom;
            // Per-channel source radii (TCA on the geometry-corrected
            // radius; green is the reference channel).
            let scales = match (&cal.tca, req.chromatic_aberration) {
                (Some(tca), true) => [
                    s_geom * sanitize_scale(tca.scale_red(r_g)),
                    s_geom,
                    s_geom * sanitize_scale(tca.scale_blue(r_g)),
                ],
                _ => [s_geom, s_geom, s_geom],
            };
            let vig = match (&cal.vignetting, req.vignette) {
                (Some(v), true) => v.falloff(r_g).max(1e-6),
                _ => 1.0,
            };
            let px = &mut out[y * w + x];
            for c in 0..3 {
                let sx = cx + dx * scales[c] * unit;
                let sy = cy + dy * scales[c] * unit;
                px[c] = (f64::from(sample(sx, sy, c)) / vig) as f32;
            }
        }
    }
    out
}

/// Hostile-data guard on model-evaluated radial scales: user-dir
/// calibration coefficients are untrusted input (the consumer-seam split
/// ruling), so a non-finite or absurd scale clamps into a sane range
/// instead of poisoning the resample coordinates.
fn sanitize_scale(s: f64) -> f64 {
    if s.is_finite() {
        s.clamp(1e-3, 1e3)
    } else {
        1.0
    }
}

// ---------------------------------------------------------------------------
// Stage 5 — highlight recovery
// ---------------------------------------------------------------------------

/// §8.3 stage 5 [NORMATIVE endpoints]: `amount = 0` clips at sensor
/// saturation; `> 0` blends reconstruction of clipped channels from the
/// unclipped ones, reaching full reconstruct-and-desaturate at 1.
///
/// Recorded election (lane log): reconstruction is per-pixel and
/// channel-based, in white-balanced value space. The clip level per
/// channel after stage 3 is `clip_c = gains[c] · 1.0` (stage-2
/// saturation × WB gain); a channel is clipped when it sits at/above
/// `clip_c · 0.995`. The neutral-highlight estimate is the MEAN of the
/// pixel's unclipped WB'd values; the full-reconstruct target is
/// `max(clip_c, est)` — clipped channels rise toward the common estimate
/// (desaturating toward neutral) and never fall below their known
/// saturation floor. All-three-clipped pixels sit at their clip levels
/// (sensor white). `amount` linearly blends hard-clip → target.
pub fn highlight_recovery(img: &mut [[f32; 3]], gains: [f32; 3], amount: f32) {
    for px in img.iter_mut() {
        let clip = gains;
        let clipped = [
            px[0] >= clip[0] * CLIP_T,
            px[1] >= clip[1] * CLIP_T,
            px[2] >= clip[2] * CLIP_T,
        ];
        if !clipped.iter().any(|c| *c) {
            continue;
        }
        let mut est = 0.0f32;
        let mut cnt = 0u32;
        for c in 0..3 {
            if !clipped[c] {
                est += px[c];
                cnt += 1;
            }
        }
        let est = if cnt > 0 { est / cnt as f32 } else { 0.0 };
        for c in 0..3 {
            if clipped[c] {
                let hard = px[c].min(clip[c]); //   amount = 0 endpoint
                let target = clip[c].max(est); //   amount = 1 endpoint
                px[c] = hard + (target - hard) * amount;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Stage 6 — camera matrix + exposure
// ---------------------------------------------------------------------------

/// Stage 6 pixel op [NORMATIVE]: `exposure_ev` as a linear gain of
/// `2^ev` (§8.3). The resolved camera matrix itself is NOT baked into
/// pixels on the fresh-document path — §43.2: commit synthesizes a linear
/// profile encoding the matrix and the codec ASSIGNS it, performing no
/// conversion; the matrix is metadata (and the custom-WB solve input).
pub fn stage6_exposure(img: &mut [[f32; 3]], exposure_ev: f32) {
    let gain = (2.0f32).powf(exposure_ev);
    for px in img.iter_mut() {
        px[0] *= gain;
        px[1] *= gain;
        px[2] *= gain;
    }
}

// ---------------------------------------------------------------------------
// Orchestration + output contract
// ---------------------------------------------------------------------------

/// The developed result heading for the §8.3 result contract (`U16`
/// linear RGB, `premultiplied = false`, fully opaque).
#[derive(Debug, Clone)]
pub struct DevelopOutput {
    pub width: usize,
    pub height: usize,
    /// Row-major interleaved RGB, `U16` linear (§43.2 output-depth rule:
    /// sensor data is ≤ 16 significant bits after stage 2, so `U16` is
    /// lossless for it; never u8, not f32 in v1).
    pub pixels: Vec<u16>,
    /// The stage-3 gains actually applied (WB mode resolved).
    pub gains: [f32; 3],
    /// §43.2 `matrixSource` for the recorded block / report item.
    pub matrix_source: MatrixSource,
}

/// Stages 3–6 over a stage-2 plane — the one develop function both
/// front-ends share (§43.1: dialog and `--opt raw.*` headless MUST be
/// this same code path). `cam_from_xyz` is the inverse resolved matrix
/// for the custom-WB solve; `None` = the §43.2 rule-3 sRGB assumption.
///
/// `lens` is the stage-4b input (§43.3 slot 4b, between demosaic and
/// highlight recovery). The v1 param surface CANNOT produce one
/// (`lensCorrection` stays a reserved key, rejected non-null): every v1
/// caller passes `None`, which is bit-identical to the pre-slice
/// pipeline. Non-`None` callers are the Post-1.0 smart-RAW layer and
/// tests ([`LensCorrection`] doc).
pub fn develop(
    cfa: &CfaPlane,
    params: &RawDevelopParams,
    as_shot_mul: [f32; 4],
    cam_from_xyz: Option<(&[[f32; 3]; 3], MatrixSource)>,
    lens: Option<&LensCorrection<'_>>,
) -> Result<DevelopOutput, CodecError> {
    // §43.3: validation BEFORE any pixel work.
    params.validate()?;
    let (inv, matrix_source) = match cam_from_xyz {
        Some((m, src)) => (*m, src),
        None => (xyz_to_assumed_cam(), MatrixSource::AssumedSrgb),
    };
    let gains = match params.white_balance {
        WhiteBalance::AsShot => as_shot_gains(as_shot_mul),
        WhiteBalance::Auto => auto_gains(cfa),
        WhiteBalance::Custom { temp_k, tint } => custom_gains(temp_k, tint, &inv),
    };
    let mut plane = cfa.clone();
    apply_wb(&mut plane, gains);
    let mut img = demosaic(&plane, params.demosaic)?;
    // Stage 4a (noise reduction): reserved Post-1.0 slot — neutral.
    // Stage 4b (lens correction): live machinery, neutral when absent
    // (§43.3 pipeline order; see [`apply_lens_correction`]).
    if let Some(req) = lens {
        img = apply_lens_correction(&img, cfa.width, cfa.height, req);
    }
    highlight_recovery(&mut img, gains, params.highlight_recovery);
    stage6_exposure(&mut img, params.exposure_ev);
    // Stage 6a (tone) reserved — the output stays linear (module doc).
    Ok(DevelopOutput {
        width: cfa.width,
        height: cfa.height,
        pixels: quantize_u16(&img),
        gains,
        matrix_source,
    })
}

/// `U16` quantization: clamp to `[0, 1]` and round half up — the fixed,
/// documented final operation (§43.4 rule 3's documented-order clause).
pub fn quantize_u16(img: &[[f32; 3]]) -> Vec<u16> {
    let mut out = Vec::with_capacity(img.len() * 3);
    for px in img {
        for v in px {
            out.push((v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic xorshift for seeded synthetic mosaics (fixture class
    /// (a) seeded synthesis, the W10/W13 corpus convention).
    struct XorShift(u64);
    impl XorShift {
        fn next_f32(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }
    }

    fn mosaic_from_rgb(img: &[[f32; 3]], w: usize, h: usize, p: CfaPattern) -> CfaPlane {
        let data = (0..w * h)
            .map(|i| img[i][p.color_at(i % w, i / w)])
            .collect();
        CfaPlane {
            width: w,
            height: h,
            pattern: p,
            data,
        }
    }

    // T-01: stage-2 normalization against hand-computed vectors.
    #[test]
    fn stage2_hand_vectors() {
        let raw: Vec<u16> = vec![512, 512, 512, 512, 2560, 4608, 16896, 16896];
        let plane =
            stage2_normalize(&raw, 4, 2, CfaPattern::Rggb, [512.0; 4], [16896.0; 4]).unwrap();
        // (512-512)/16384 = 0; (2560-512)/16384 = 0.125;
        // (4608-512)/16384 = 0.25; (16896-512)/16384 = 1.0.
        assert_eq!(plane.data[0], 0.0);
        assert_eq!(plane.data[4], 0.125);
        assert_eq!(plane.data[5], 0.25);
        assert_eq!(plane.data[6], 1.0);
    }

    #[test]
    fn stage2_clamps_below_black_at_zero() {
        let plane =
            stage2_normalize(&[100], 1, 1, CfaPattern::Rggb, [512.0; 4], [16896.0; 4]).unwrap();
        assert_eq!(plane.data[0], 0.0);
    }

    // T-01: 2^ev gain placement.
    #[test]
    fn exposure_gain_is_pow2() {
        let mut img = vec![[0.125f32, 0.2, 0.05]];
        stage6_exposure(&mut img, 2.0);
        assert_eq!(img[0], [0.5, 0.8, 0.2]);
        stage6_exposure(&mut img, -1.0);
        assert_eq!(img[0], [0.25, 0.4, 0.1]);
    }

    #[test]
    fn as_shot_gains_normalize_green_to_one() {
        let g = as_shot_gains([2.0, 1.0, 1.5, 1.0]);
        assert_eq!(g, [2.0, 1.0, 1.5]);
        // g2 = 0 means "same as g1" (LibRaw convention).
        let g = as_shot_gains([2.0, 2.0, 3.0, 0.0]);
        assert_eq!(g, [1.0, 1.0, 1.5]);
    }

    // T-01: CCT→xy custom-WB conversion, behavioral pins.
    #[test]
    fn custom_wb_gains_behave_physically() {
        let inv = xyz_to_assumed_cam();
        // D65-ish: near-neutral gains under the sRGB assumption.
        let g65 = custom_gains(6504.0, 0.0, &inv);
        assert_eq!(g65[1], 1.0);
        assert!((g65[0] - 1.0).abs() < 0.05, "r gain {g65:?}");
        assert!((g65[2] - 1.0).abs() < 0.05, "b gain {g65:?}");
        // Tungsten (3000 K): the scene is red-heavy, so compensation puts
        // MORE gain on blue than red.
        let g30 = custom_gains(3000.0, 0.0, &inv);
        assert!(g30[2] > g30[0], "tungsten gains {g30:?}");
        // Shade (10000 K): the opposite ordering.
        let g100 = custom_gains(10_000.0, 0.0, &inv);
        assert!(g100[0] > g100[2], "shade gains {g100:?}");
        // Tint moves red and blue the same way relative to green
        // (magenta⇄green axis), monotonically.
        let gp = custom_gains(6504.0, 100.0, &inv);
        let gm = custom_gains(6504.0, -100.0, &inv);
        assert!(gp[0] != gm[0] && gp[2] != gm[2]);
        assert!(
            (gp[0] - gm[0]).signum() == (gp[2] - gm[2]).signum(),
            "tint must not be an r/b see-saw: {gp:?} vs {gm:?}"
        );
    }

    // §43.4 rule 2 structure: banded f64 partials == one-pass reference.
    #[test]
    fn auto_wb_band_merge_matches_flat_reference() {
        let (w, h) = (32, BAND_ROWS * 2 + 57); // spans 3 bands, ragged tail
        let mut rng = XorShift(0xB060_5020);
        let data: Vec<f32> = (0..w * h).map(|_| rng.next_f32() * 0.9).collect();
        let cfa = CfaPlane {
            width: w,
            height: h,
            pattern: CfaPattern::Rggb,
            data,
        };
        let banded = auto_gains(&cfa);
        // Flat reference: same math, one band the height of the image.
        let mut acc = [[0.0f64; 2]; 3];
        for y in 0..h {
            let wy = 1.0 - ((2.0 * y as f64) / ((h - 1) as f64) - 1.0).abs();
            for x in 0..w {
                let v = cfa.data[y * w + x];
                if v >= 0.995 {
                    continue;
                }
                let wx = 1.0 - ((2.0 * x as f64) / ((w - 1) as f64) - 1.0).abs();
                let c = cfa.pattern.color_at(x, y);
                acc[c][0] += f64::from(v) * wx * wy;
                acc[c][1] += wx * wy;
            }
        }
        let m = |c: usize| acc[c][0] / acc[c][1];
        let expect = [(m(1) / m(0)) as f32, 1.0, (m(1) / m(2)) as f32];
        // f64 band partials summed in ascending order vs flat: tiny
        // reassociation slack only.
        for c in 0..3 {
            assert!(
                (banded[c] - expect[c]).abs() < 1e-5,
                "{banded:?} vs {expect:?}"
            );
        }
    }

    #[test]
    fn auto_wb_neutralizes_a_uniform_cast() {
        // A flat gray scene shot through a red cast: R sites read 0.6,
        // G/B sites 0.3 → auto gains must bring R down to G's mean.
        let (w, h) = (16, 16);
        let data: Vec<f32> = (0..w * h)
            .map(|i| {
                if CfaPattern::Rggb.color_at(i % w, i / w) == 0 {
                    0.6
                } else {
                    0.3
                }
            })
            .collect();
        let cfa = CfaPlane {
            width: w,
            height: h,
            pattern: CfaPattern::Rggb,
            data,
        };
        let g = auto_gains(&cfa);
        assert!((g[0] - 0.5).abs() < 1e-6, "{g:?}");
        assert!((g[2] - 1.0).abs() < 1e-6, "{g:?}");
    }

    // Dossier acceptance: prop demosaic_gray_world_neutrality.
    #[test]
    fn demosaic_gray_world_neutrality() {
        for pattern in [
            CfaPattern::Rggb,
            CfaPattern::Bggr,
            CfaPattern::Grbg,
            CfaPattern::Gbrg,
        ] {
            let (w, h) = (12, 10);
            let cfa = CfaPlane {
                width: w,
                height: h,
                pattern,
                data: vec![0.37f32; w * h],
            };
            for algo in [Demosaic::Linear, Demosaic::Ahd] {
                let img = demosaic(&cfa, algo).unwrap();
                for px in &img {
                    for c in 0..3 {
                        assert!(
                            (px[c] - 0.37).abs() < 1e-5,
                            "{pattern:?}/{algo:?} broke neutrality: {px:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn demosaic_keeps_measured_photosites_verbatim() {
        let (w, h) = (8, 8);
        let mut rng = XorShift(42);
        let data: Vec<f32> = (0..w * h).map(|_| rng.next_f32()).collect();
        let cfa = CfaPlane {
            width: w,
            height: h,
            pattern: CfaPattern::Rggb,
            data,
        };
        let img = demosaic_bilinear(&cfa);
        for y in 0..h {
            for x in 0..w {
                let c = cfa.pattern.color_at(x, y);
                assert_eq!(img[y * w + x][c], cfa.data[y * w + x]);
            }
        }
    }

    /// AHD earns its default (dossier benchmark direction): on an image
    /// with strong vertical and horizontal edges, AHD reconstructs with
    /// lower MSE than bilinear.
    #[test]
    fn ahd_beats_bilinear_on_edges() {
        let (w, h) = (48, 48);
        let mut truth = vec![[0.0f32; 3]; w * h];
        for y in 0..h {
            for x in 0..w {
                // Luminance bars + a horizontal band under near-constant
                // chroma — the structure AHD's directional homogeneity
                // vote exists for (dossier zipper metric direction).
                let bar = if (x / 6) % 2 == 0 { 0.85 } else { 0.15 };
                let v = if (16..24).contains(&y) { 0.7 } else { bar };
                truth[y * w + x] = [v, (v * 0.96 + 0.02).min(1.0), v * 0.9 + 0.05];
            }
        }
        let cfa = mosaic_from_rgb(&truth, w, h, CfaPattern::Rggb);
        let mse = |img: &[[f32; 3]]| -> f64 {
            // Interior only: the border rule differs from real content.
            let mut e = 0.0f64;
            let mut n = 0u32;
            for y in 4..h - 4 {
                for x in 4..w - 4 {
                    for c in 0..3 {
                        let d = f64::from(img[y * w + x][c] - truth[y * w + x][c]);
                        e += d * d;
                        n += 1;
                    }
                }
            }
            e / f64::from(n)
        };
        let e_lin = mse(&demosaic_bilinear(&cfa));
        let e_ahd = mse(&demosaic_ahd(&cfa));
        assert!(
            e_ahd < e_lin,
            "AHD mse {e_ahd:.6} must beat bilinear {e_lin:.6}"
        );
    }

    #[test]
    fn vng_ppg_are_typed_deferrals() {
        let cfa = CfaPlane {
            width: 4,
            height: 4,
            pattern: CfaPattern::Rggb,
            data: vec![0.5; 16],
        };
        for algo in [Demosaic::Vng, Demosaic::Ppg] {
            match demosaic(&cfa, algo) {
                Err(CodecError::UnsupportedFeature { feature }) => {
                    assert!(feature.contains("pending_slice"), "{feature}");
                }
                other => panic!("expected typed deferral, got {other:?}"),
            }
        }
    }

    // T-01: highlight-recovery endpoints (0 = clip, 1 = full reconstruct).
    #[test]
    fn highlight_recovery_endpoints() {
        let gains = [2.0f32, 1.0, 1.5];
        // R clipped (pre-WB 1.0 × gain 2.0), G/B unclipped at 60 %.
        let base = [2.0f32, 0.6, 0.9];
        let mut at0 = vec![base];
        highlight_recovery(&mut at0, gains, 0.0);
        assert_eq!(at0[0], [2.0, 0.6, 0.9], "amount 0 = clip at saturation");
        let mut over = vec![[2.4f32, 0.6, 0.9]];
        highlight_recovery(&mut over, gains, 0.0);
        assert_eq!(over[0][0], 2.0, "amount 0 clips overshoot to clip level");
        let mut at1 = vec![base];
        highlight_recovery(&mut at1, gains, 1.0);
        // est = mean(0.6, 0.9) = 0.75 < clip_r = 2.0 → the saturation
        // floor holds: unclipped channels darker than clip keep R at clip.
        assert_eq!(at1[0], [2.0, 0.6, 0.9]);
        // A channel whose clip level sits BELOW the unclipped mean is
        // reconstructed upward toward it (and only at amount = 1 fully).
        let gains2 = [0.8f32, 1.0, 1.2];
        let mut hot = vec![[0.85f32, 0.95, 0.9]]; // r sensor-clipped
        highlight_recovery(&mut hot, gains2, 1.0);
        let est = 0.5 * (0.95 + 0.9);
        assert!((hot[0][0] - est).abs() < 1e-6, "{:?}", hot[0]);
        let mut half = vec![[0.85f32, 0.95, 0.9]];
        highlight_recovery(&mut half, gains2, 0.5);
        assert!(
            (half[0][0] - 0.5 * (0.8 + est)).abs() < 1e-6,
            "{:?}",
            half[0]
        );
    }

    #[test]
    fn highlight_recovery_desaturates_when_all_clip() {
        let gains = [2.0f32, 1.0, 1.5];
        let mut px = vec![[2.3f32, 1.2, 1.7]];
        highlight_recovery(&mut px, gains, 1.0);
        // All clipped → clip levels exactly → neutral once WB divides out.
        assert_eq!(px[0], [2.0, 1.0, 1.5]);
    }

    #[test]
    fn quantize_clamps_and_rounds() {
        let q = quantize_u16(&[[0.0, 1.0, 2.0], [-0.5, 0.5, 0.25]]);
        assert_eq!(q, [0, 65535, 65535, 0, 32768, 16384]);
    }

    /// End-to-end determinism at the develop() level: identical inputs ⇒
    /// bit-identical U16 output, repeated (§43.4 pure-function claim, the
    /// single-machine leg the unit tier can honestly assert).
    #[test]
    fn develop_is_bit_stable_across_runs() {
        let (w, h) = (24, 20);
        let mut rng = XorShift(0xB060_5021);
        let raw: Vec<u16> = (0..w * h)
            .map(|_| (rng.next_f32() * 16000.0) as u16)
            .collect();
        let cfa = stage2_normalize(&raw, w, h, CfaPattern::Grbg, [256.0; 4], [15872.0; 4]).unwrap();
        let params = RawDevelopParams {
            white_balance: WhiteBalance::Custom {
                temp_k: 5500.0,
                tint: 10.0,
            },
            exposure_ev: 0.5,
            highlight_recovery: 0.25,
            ..Default::default()
        };
        let a = develop(&cfa, &params, [2.0, 1.0, 1.4, 1.0], None, None).unwrap();
        let b = develop(&cfa, &params, [2.0, 1.0, 1.4, 1.0], None, None).unwrap();
        assert_eq!(a.pixels, b.pixels);
        assert_eq!(a.gains, b.gains);
        assert_eq!(a.matrix_source, MatrixSource::AssumedSrgb);
        assert_eq!(a.pixels.len(), w * h * 3);
    }

    #[test]
    fn develop_validates_before_pixel_work() {
        let cfa = CfaPlane {
            width: 4,
            height: 4,
            pattern: CfaPattern::Rggb,
            data: vec![0.5; 16],
        };
        let params = RawDevelopParams {
            exposure_ev: 11.0,
            ..Default::default()
        };
        assert!(matches!(
            develop(&cfa, &params, [1.0; 4], None, None),
            Err(CodecError::OptionOutOfRange { .. })
        ));
    }

    // ------ RAW-binding slice: bound advisory 1 (D50/D65 ICC colorants) ------

    /// The advisory's trap, pinned: the D65-native assumption matrix is
    /// NOT the ICC colorant matrix — stage-7 colorants are the Bradford
    /// D50 adaptation, which measurably differs.
    #[test]
    fn advisory_d50_colorants_differ_from_d65_constant() {
        let d50 = stage7_colorants_d50(&SRGB_TO_XYZ);
        let mut max_diff = 0.0f32;
        for i in 0..3 {
            for j in 0..3 {
                max_diff = max_diff.max((d50[i][j] - SRGB_TO_XYZ[i][j]).abs());
            }
        }
        assert!(
            max_diff > 0.01,
            "D50 adaptation must move the colorants ({max_diff})"
        );
        // White point: columns sum to the D50 white, not D65.
        for (row, want) in WHITE_D50.iter().enumerate() {
            let sum = d50[row][0] + d50[row][1] + d50[row][2];
            assert!(
                (f64::from(sum) - want).abs() < 2e-4,
                "row {row}: colorant sum {sum} vs D50 {want}"
            );
        }
    }

    /// Independent oracle: appname-color's own §5.5 sRGB → XYZ(D50)
    /// matrix (the icc_synth colorant source already on main) must agree
    /// with this crate's Bradford adaptation of the D65 constant.
    #[test]
    fn advisory_d50_colorants_match_appname_color_oracle() {
        let ours = stage7_colorants_d50(&SRGB_TO_XYZ);
        let oracle = appname_color::BuiltinSpace::Srgb
            .rgb_to_xyz_d50()
            .expect("sRGB has a D50 matrix")
            .m;
        for i in 0..3 {
            for j in 0..3 {
                assert!(
                    (f64::from(ours[i][j]) - oracle[i][j]).abs() < 2e-3,
                    "[{i}][{j}] ours {} vs oracle {}",
                    ours[i][j],
                    oracle[i][j]
                );
            }
        }
    }

    /// The published sRGB D50 colorant check values (ICC v4 sRGB red
    /// column ≈ (0.4360, 0.2225, 0.0139)).
    #[test]
    fn advisory_d50_red_colorant_matches_published_value() {
        let d50 = stage7_colorants_d50(&SRGB_TO_XYZ);
        assert!((d50[0][0] - 0.4360).abs() < 2e-3, "{}", d50[0][0]);
        assert!((d50[1][0] - 0.2225).abs() < 2e-3, "{}", d50[1][0]);
        assert!((d50[2][0] - 0.0139).abs() < 2e-3, "{}", d50[2][0]);
    }

    // ------ RAW-binding slice: bound advisory 2 (degenerate levels) ------

    /// The advisory's exact panic path, now typed: white == black used to
    /// propagate non-finite through stage 2 into a median9 panic under
    /// AHD. It must be a typed error.codec.corrupt instead.
    #[test]
    fn advisory_degenerate_levels_are_typed_not_panic() {
        let raw: Vec<u16> = vec![512; 8 * 8];
        let err = stage2_normalize(&raw, 8, 8, CfaPattern::Rggb, [512.0; 4], [512.0; 4])
            .expect_err("white == black must refuse");
        assert_eq!(err.message_id(), "error.codec.corrupt");
        assert!(err.to_string().contains("levels_degenerate"), "{err}");
        // Non-finite levels: same arm.
        let err = stage2_normalize(&raw, 8, 8, CfaPattern::Rggb, [f32::NAN; 4], [16000.0; 4])
            .expect_err("NaN black must refuse");
        assert_eq!(err.message_id(), "error.codec.corrupt");
        // White below black: same arm.
        assert!(stage2_normalize(&raw, 8, 8, CfaPattern::Rggb, [600.0; 4], [500.0; 4]).is_err());
    }

    /// Per-position sanitizer surface (the stage-1 wrapper's early call).
    #[test]
    fn sanitize_levels_names_the_offending_position() {
        let err = sanitize_levels([0.0, 0.0, 7.0, 0.0], [100.0, 100.0, 7.0, 100.0]).unwrap_err();
        assert!(err.to_string().contains("position 2"), "{err}");
        sanitize_levels([0.0; 4], [100.0; 4]).unwrap();
    }

    /// median9 is panic-free even on hostile values (defense in depth
    /// behind the sanitizer).
    #[test]
    fn median9_survives_non_finite() {
        let mut v = [
            f32::NAN,
            1.0,
            f32::INFINITY,
            0.5,
            -1.0,
            f32::NEG_INFINITY,
            0.0,
            2.0,
            f32::NAN,
        ];
        let _ = median9(&mut v); // must not panic
    }

    // ------ RAW-binding slice: stage 4b (lens correction machinery) ------

    use appname_algo::lensdb::{
        DistortionModel, ResolvedCalibration, ResolvedVignetting, TcaModel,
    };

    fn checker(w: usize, h: usize) -> Vec<[f32; 3]> {
        (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let v = if (x + y) % 2 == 0 { 0.8 } else { 0.2 };
                [v, v * 0.9, v * 0.8]
            })
            .collect()
    }

    fn full_req(cal: &ResolvedCalibration) -> LensCorrection<'_> {
        LensCorrection {
            calibration: cal,
            distortion: true,
            chromatic_aberration: true,
            vignette: true,
        }
    }

    /// Identity models leave the image bit-identical (sampling at exact
    /// grid points is exact — pins the resampler's identity behavior).
    #[test]
    fn lens_identity_calibration_is_bit_exact() {
        let (w, h) = (16, 12);
        let img = checker(w, h);
        let cal = ResolvedCalibration {
            distortion: Some(DistortionModel::Poly5 { k1: 0.0, k2: 0.0 }),
            tca: Some(TcaModel::Linear { kr: 1.0, kb: 1.0 }),
            vignetting: Some(ResolvedVignetting {
                k1: 0.0,
                k2: 0.0,
                k3: 0.0,
                aperture: 4.0,
                distance: 1000.0,
            }),
        };
        let out = apply_lens_correction(&img, w, h, &full_req(&cal));
        assert_eq!(out, img);
    }

    /// Toggles off = that effect absent even when calibrated (the §43.3
    /// reserved-schema boolean semantics).
    #[test]
    fn lens_toggles_gate_each_effect() {
        let (w, h) = (16, 12);
        let img = checker(w, h);
        let cal = ResolvedCalibration {
            distortion: Some(DistortionModel::Poly5 { k1: -0.2, k2: 0.0 }),
            tca: None,
            vignetting: Some(ResolvedVignetting {
                k1: -0.3,
                k2: 0.0,
                k3: 0.0,
                aperture: 4.0,
                distance: 1000.0,
            }),
        };
        let all_off = LensCorrection {
            calibration: &cal,
            distortion: false,
            chromatic_aberration: false,
            vignette: false,
        };
        assert_eq!(apply_lens_correction(&img, w, h, &all_off), img);
        let vig_only = LensCorrection {
            calibration: &cal,
            distortion: false,
            chromatic_aberration: false,
            vignette: true,
        };
        let out = apply_lens_correction(&img, w, h, &vig_only);
        assert_ne!(out, img);
    }

    /// Vignetting correction divides by the falloff at the pixel's
    /// radius — hand-checked at the exact-grid corner of a flat image.
    #[test]
    fn lens_vignette_divides_by_falloff() {
        let (w, h) = (9, 9); // odd => center and corners at integer px
        let img = vec![[0.5f32, 0.5, 0.5]; w * h];
        let cal = ResolvedCalibration {
            distortion: None,
            tca: None,
            vignetting: Some(ResolvedVignetting {
                k1: -0.2,
                k2: 0.0,
                k3: 0.0,
                aperture: 4.0,
                distance: 1000.0,
            }),
        };
        let out = apply_lens_correction(
            &img,
            w,
            h,
            &LensCorrection {
                calibration: &cal,
                distortion: false,
                chromatic_aberration: false,
                vignette: true,
            },
        );
        // Center: r = 0, falloff = 1 => unchanged.
        assert!((out[4 * w + 4][0] - 0.5).abs() < 1e-7);
        // Corner (0,0): r^2 = 2 under the min-dimension anchor
        // (dx = dy = -1), falloff = 1 - 0.2*2 = 0.6.
        let want = 0.5 / 0.6;
        assert!((out[0][0] - want).abs() < 1e-6, "{} vs {want}", out[0][0]);
    }

    /// Barrel-distortion source mapping, hand-checked at an exact point:
    /// output pixel at r_u samples r_u * scale(r_u).
    #[test]
    fn lens_distortion_samples_the_scaled_radius() {
        let (w, h) = (9, 9);
        // Horizontal gradient: value = x / 8 (all channels).
        let img: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let x = (i % w) as f32 / 8.0;
                [x, x, x]
            })
            .collect();
        let cal = ResolvedCalibration {
            // scale(r) = 1 + 0.1 r^2 (poly5) => at (8,4): dx=1, dy=0,
            // r_u=1, scale=1.1 => source x = 4 + 1*1.1*4 = 8.4 -> clamped
            // bilinear at 8.0 edge = 1.0? No: clamp keeps inside, 8.4
            // clamps to 8 => value 1.0. Use an interior point instead:
            // (6,4): dx = 0.5, r_u = 0.5, scale = 1.025, source x =
            // 4 + 0.5*1.025*4 = 6.05 => value 6.05/8.
            distortion: Some(DistortionModel::Poly5 { k1: 0.1, k2: 0.0 }),
            tca: None,
            vignetting: None,
        };
        let out = apply_lens_correction(
            &img,
            w,
            h,
            &LensCorrection {
                calibration: &cal,
                distortion: true,
                chromatic_aberration: false,
                vignette: false,
            },
        );
        let want = 6.05f32 / 8.0;
        assert!(
            (out[4 * w + 6][1] - want).abs() < 1e-6,
            "{} vs {want}",
            out[4 * w + 6][1]
        );
    }

    /// TCA scales only the toggled channels, green untouched.
    #[test]
    fn lens_tca_scales_red_blue_not_green() {
        let (w, h) = (9, 9);
        let img: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let x = (i % w) as f32 / 8.0;
                [x, x, x]
            })
            .collect();
        let cal = ResolvedCalibration {
            distortion: None,
            tca: Some(TcaModel::Linear { kr: 1.05, kb: 0.95 }),
            vignetting: None,
        };
        let out = apply_lens_correction(
            &img,
            w,
            h,
            &LensCorrection {
                calibration: &cal,
                distortion: false,
                chromatic_aberration: true,
                vignette: false,
            },
        );
        let px = out[4 * w + 6];
        // Green: identity => 6/8.
        assert!((px[1] - 0.75).abs() < 1e-7);
        // Red: source x = 4 + 0.5*1.05*4 = 6.1 => 6.1/8.
        assert!((px[0] - 6.1 / 8.0).abs() < 1e-6, "{}", px[0]);
        // Blue: source x = 4 + 0.5*0.95*4 = 5.9 => 5.9/8.
        assert!((px[2] - 5.9 / 8.0).abs() < 1e-6, "{}", px[2]);
    }

    /// Hostile user-dir coefficients (non-finite, absurd) never produce
    /// non-finite output (consumer-seam untrusted-input discipline).
    #[test]
    fn lens_hostile_coefficients_stay_finite() {
        let (w, h) = (8, 8);
        let img = checker(w, h);
        let cal = ResolvedCalibration {
            distortion: Some(DistortionModel::Ptlens {
                a: f64::NAN,
                b: 1e18,
                c: -1e18,
            }),
            tca: Some(TcaModel::Poly3 {
                vr: f64::INFINITY,
                vb: -1e12,
                cr: f64::NAN,
                cb: 0.0,
                br: 1e9,
                bb: f64::NEG_INFINITY,
            }),
            vignetting: Some(ResolvedVignetting {
                k1: -1e9,
                k2: f64::NAN,
                k3: 0.0,
                aperture: 4.0,
                distance: 1000.0,
            }),
        };
        let out = apply_lens_correction(&img, w, h, &full_req(&cal));
        for px in &out {
            for c in 0..3 {
                assert!(px[c].is_finite(), "{px:?}");
            }
        }
    }

    /// develop() with a lens request runs 4b in the pinned §43.3 slot:
    /// identical to manually correcting between demosaic and highlight
    /// recovery.
    #[test]
    fn develop_runs_stage_4b_in_slot_order() {
        let (w, h) = (16, 12);
        let mut rng = XorShift(0xB060_5022);
        let raw: Vec<u16> = (0..w * h)
            .map(|_| (rng.next_f32() * 15000.0) as u16)
            .collect();
        let cfa = stage2_normalize(&raw, w, h, CfaPattern::Rggb, [256.0; 4], [15872.0; 4]).unwrap();
        let params = RawDevelopParams {
            exposure_ev: 0.5,
            highlight_recovery: 0.5,
            ..Default::default()
        };
        let cal = ResolvedCalibration {
            distortion: Some(DistortionModel::Poly3 { k1: -0.05 }),
            tca: None,
            vignetting: Some(ResolvedVignetting {
                k1: -0.1,
                k2: 0.0,
                k3: 0.0,
                aperture: 2.8,
                distance: 1000.0,
            }),
        };
        let req = full_req(&cal);
        let with_lens = develop(&cfa, &params, [2.0, 1.0, 1.4, 1.0], None, Some(&req)).unwrap();

        // Manual reference: WB -> demosaic -> 4b -> highlight -> 2^ev.
        let gains = as_shot_gains([2.0, 1.0, 1.4, 1.0]);
        let mut plane = cfa.clone();
        apply_wb(&mut plane, gains);
        let mut img = demosaic(&plane, params.demosaic).unwrap();
        img = apply_lens_correction(&img, w, h, &req);
        highlight_recovery(&mut img, gains, params.highlight_recovery);
        stage6_exposure(&mut img, params.exposure_ev);
        assert_eq!(with_lens.pixels, quantize_u16(&img));
        // And None stays the pre-slice pipeline (bit-compat pin).
        let without = develop(&cfa, &params, [2.0, 1.0, 1.4, 1.0], None, None).unwrap();
        assert_ne!(without.pixels, with_lens.pixels);
    }
}
