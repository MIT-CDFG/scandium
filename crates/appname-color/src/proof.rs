// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §5.6 (soft proofing and gamut warning — the `SoftProof`
//! struct verbatim; "The proof chain and gamut flag, precisely": the
//! rgba proof LUT, the [NORMATIVE] out-of-gamut criterion — round-trip
//! working→proof→working error ΔE76 > 2.0, threshold not
//! user-configurable; paper-white simulation = absolute colorimetric on
//! the proof→display leg with BPC off), §5.4 (the 33³ LUT, the
//! [NORMATIVE] exact bake loop — ENCODED working-space domain, red
//! fastest, memory index `((b·N + g)·N + r)`; trilinear interpolation
//! for display/proof LUTs; invalidation triggers), §29.3 (the print
//! dialog preview IS this soft proof); DD ⑦.
//!
//! Color-side §5.6: the proof/display LUT bake and the CPU reference
//! sampler. The chain is `working → proof(intent, bpc) → display`, baked
//! leg-by-leg through ordinary cached [`crate::Cms`] transforms (each
//! leg `cmsFLAGS_NOCACHE`, batched over the whole lattice — "one batched
//! cmsDoTransform" per leg), with the gamut flag computed per lattice
//! point by the normative round-trip in Lab (through the builtin Lab
//! connection-space profile — the same real-CMM math as the pixels).
//!
//! ## The display-chain integration point (named for O-UI)
//! The renderer's present pass consumes exactly one of:
//! * [`DisplayLutData`] (no proof): rgb 3D LUT, trilinear, from
//!   [`bake_display_lut`] — the §5.4 `bake_display_lut(working, display,
//!   None, …)` arm;
//! * [`ProofLut`] (soft proof on): **rgba** 3D LUT from
//!   [`bake_proof_lut`] — "the proof LUT replaces the display LUT", rgb
//!   = display-encoded proofed color, a = gamut flag; the shader ORs the
//!   user's warning color where `a > 0.5` when
//!   [`SoftProof::gamut_warning`] is set (§5.6 WGSL sketch; the CPU
//!   reference is [`sample_lut3d_rgba`] + [`gamut_flagged`]).
//!
//! Input domain both ways: **encoded** working RGB (§5.4 [NORMATIVE]),
//! `x = R` fastest. Re-bake triggers are the §5.4 list (working/monitor
//! profile change, any `SoftProof` field change, intent/BPC preference,
//! mode change); per-monitor keying (`MonitorId`) is view state and
//! lives with the consumer — these LUTs are keyed by their profile
//! uids + params alone.
//!
//! v1 posture (recorded in the lane log): the bake requires an
//! RGB-data-space working profile — the viewport chain of CP-1.0 proofs
//! RGB working documents; Gray/CMYK/Lab-working viewports ride the same
//! legs but need their own lattice shapes (a 1-D/4-D domain), reserved
//! with the display-chain door.

use std::sync::Arc;

use crate::error::ColorError;
use crate::lab::{delta_e76, lab_from_normalized};
use crate::profile::{DataColorSpace, ProfileUid};
use crate::transform::{Cms, Transform, TransformKey};
use crate::types::{ColorModel, Depth, Intent, PixelFormat};

/// §5.6 `SoftProof`, verbatim shape (profile identity in this crate's
/// content-keyed vocabulary).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SoftProof {
    pub proof_profile: ProfileUid,
    pub intent: Intent,
    pub bpc: bool,
    pub simulate_paper: bool,
    pub gamut_warning: bool,
}

/// §5.6 [NORMATIVE]: the out-of-gamut criterion threshold, ΔE76 = 2.0 —
/// deliberately CIE76 (the §5.5 warnings-vs-tests split) and NOT
/// user-configurable.
pub const GAMUT_WARNING_THRESHOLD_DE76: f64 = 2.0;

/// §5.4: display and soft-proof transforms bake into a 33³ lattice.
pub const LUT_SIZE: usize = 33;

/// The §5.4 display LUT payload (rgb), profile-keyed; the consumer
/// attaches its `MonitorId` when binding (§5.7 is view state).
#[derive(Clone, PartialEq, Debug)]
pub struct DisplayLutData {
    pub size: u32,
    /// `size³` entries, red fastest: index `((b·N + g)·N + r)`.
    pub data: Vec<[f32; 3]>,
}

/// The §5.6 proof LUT payload (**rgba**): rgb = display-encoded result
/// of `working → proof(intent, bpc) → display`, a = the gamut flag.
#[derive(Clone, PartialEq, Debug)]
pub struct ProofLut {
    pub size: u32,
    /// `size³` entries, red fastest: index `((b·N + g)·N + r)`.
    pub data: Vec<[f32; 4]>,
}

/// The §5.4 lattice: encoded working RGB, red fastest, `N³ × 3`
/// interleaved f32 (the exact bake-loop shape).
fn lattice(n: usize) -> Vec<f32> {
    let mut src = vec![0f32; n * n * n * 3];
    let mut i = 0;
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                // r innermost: red fastest
                src[i] = r as f32 / (n - 1) as f32; // encoded working value
                src[i + 1] = g as f32 / (n - 1) as f32;
                src[i + 2] = b as f32 / (n - 1) as f32;
                i += 3;
            }
        }
    }
    src
}

fn rgb_f32(model: ColorModel) -> PixelFormat {
    PixelFormat::new(model, Depth::F32)
}

/// Resolve a profile's alpha-less CMS model from its data space (the
/// §5.6 proof profile may be RGB, Gray, CMYK, or Lab class — "typically
/// Output class but any of the four is accepted", §5.2 rule 3).
fn model_of(cms: &Cms, uid: ProfileUid) -> Result<ColorModel, ColorError> {
    Ok(match cms.registry().get(uid)?.data_space {
        DataColorSpace::Rgb => ColorModel::Rgb,
        DataColorSpace::Gray => ColorModel::Gray,
        DataColorSpace::Cmyk => ColorModel::Cmyk,
        DataColorSpace::Lab => ColorModel::Lab,
        _ => return Err(ColorError::ProfileSpaceMismatch),
    })
}

fn require_rgb(cms: &Cms, uid: ProfileUid, what: &'static str) -> Result<(), ColorError> {
    if cms.registry().get(uid)?.data_space != DataColorSpace::Rgb {
        return Err(ColorError::TransformBuild(what));
    }
    Ok(())
}

fn leg(
    cms: &mut Cms,
    src: ProfileUid,
    dst: ProfileUid,
    model_in: ColorModel,
    model_out: ColorModel,
    intent: Intent,
    bpc: bool,
) -> Result<Arc<Transform>, ColorError> {
    cms.transform(TransformKey {
        src,
        dst,
        proof: None,
        intent,
        bpc,
        fmt_in: rgb_f32(model_in),
        fmt_out: rgb_f32(model_out),
    })
}

/// §5.4 `bake_display_lut`, the no-proof arm: bake
/// `working → display(intent, bpc)` over the encoded-working lattice.
/// One batched apply (the legs are `NOCACHE`; batching cannot change
/// bytes). [PERF §5.4] budget: < 50 ms on the reference machine — it
/// sits on the monitor hot-plug path (measured by the perf harness, not
/// asserted here).
pub fn bake_display_lut(
    cms: &mut Cms,
    working: ProfileUid,
    display: ProfileUid,
    intent: Intent,
    bpc: bool,
) -> Result<DisplayLutData, ColorError> {
    require_rgb(
        cms,
        working,
        "display-LUT bake requires an RGB working space (v1 posture)",
    )?;
    require_rgb(cms, display, "display profiles are RGB-class")?;
    let t = leg(
        cms,
        working,
        display,
        ColorModel::Rgb,
        ColorModel::Rgb,
        intent,
        bpc,
    )?;
    let src = lattice(LUT_SIZE);
    let mut out = vec![0f32; src.len()];
    t.apply_f32(&src, &mut out)?;
    let data = out
        .chunks_exact(3)
        .map(|c| [c[0], c[1], c[2]])
        .collect::<Vec<_>>();
    Ok(DisplayLutData {
        size: LUT_SIZE as u32,
        data,
    })
}

/// §5.6 "The proof chain and gamut flag, precisely" — the exact bake:
///
/// ```text
/// for each lattice point w (encoded working RGB):
///     p    = transform(w,  working → proof,   proof.intent, proof.bpc)
///     back = transform(p,  proof   → working, relative colorimetric, bpc = false)
///     flag = ΔE76( Lab(w), Lab(back) ) > 2.0  ? 1.0 : 0.0
///     lut[w] = ( display_encode(working→proof→display applied to w),  flag )
/// ```
///
/// The proof→display leg runs relative colorimetric (BPC off — the §5.6
/// letter fixes the leg's intent; the user's `bpc` belongs to the
/// working→proof leg), or **absolute colorimetric** when
/// `simulate_paper` (paper-white simulation; lcms2 performs the
/// media-white scaling internally — "Appname adds no ad-hoc scaling" —
/// and paper simulation implies black-ink simulation, BPC off). `Lab(·)`
/// runs through the builtin Lab connection-space profile at relative
/// colorimetric, no BPC — the same CMM math as the pixels.
pub fn bake_proof_lut(
    cms: &mut Cms,
    working: ProfileUid,
    display: ProfileUid,
    proof: &SoftProof,
) -> Result<ProofLut, ColorError> {
    require_rgb(
        cms,
        working,
        "proof bake requires an RGB working space (v1 posture)",
    )?;
    require_rgb(cms, display, "display profiles are RGB-class")?;
    let proof_model = model_of(cms, proof.proof_profile)?;
    let lab = cms.builtin_lab();

    let t_wp = leg(
        cms,
        working,
        proof.proof_profile,
        ColorModel::Rgb,
        proof_model,
        proof.intent,
        proof.bpc,
    )?;
    let display_intent = if proof.simulate_paper {
        Intent::AbsoluteColorimetric
    } else {
        Intent::RelativeColorimetric
    };
    let t_pd = leg(
        cms,
        proof.proof_profile,
        display,
        proof_model,
        ColorModel::Rgb,
        display_intent,
        false,
    )?;
    let t_pw = leg(
        cms,
        proof.proof_profile,
        working,
        proof_model,
        ColorModel::Rgb,
        Intent::RelativeColorimetric,
        false,
    )?;
    let t_w_lab = leg(
        cms,
        working,
        lab,
        ColorModel::Rgb,
        ColorModel::Lab,
        Intent::RelativeColorimetric,
        false,
    )?;

    let n = LUT_SIZE;
    let count = n * n * n;
    let w = lattice(n);
    let pch = proof_model.channels();
    // Leg 1: working → proof (whole lattice, one batched apply).
    let mut p = vec![0f32; count * pch];
    t_wp.apply_f32(&w, &mut p)?;
    // Device-range clamp, recorded election: lcms2 float paths are
    // UNBOUNDED for matrix/shaper-class profiles (no gamut clipping —
    // out-of-gamut working colors land outside [0,1] in proof encoding
    // and would round-trip back losslessly, silently defeating the §5.6
    // criterion). A physical proof device cannot represent outside its
    // encoding range, so the proof point is clamped to it before the
    // display and round-trip legs; CLUT-class Output profiles clip
    // internally and the clamp is then a no-op. (Normalized encoding for
    // every model here — the Lab/CMYK real-value marshalling lives
    // inside the transform boundary, icc.rs.)
    for v in &mut p {
        *v = v.clamp(0.0, 1.0);
    }
    // Leg 2: proof → display (the LUT rgb).
    let mut d = vec![0f32; count * 3];
    t_pd.apply_f32(&p, &mut d)?;
    // Leg 3: proof → working (the round trip for the flag).
    let mut back = vec![0f32; count * 3];
    t_pw.apply_f32(&p, &mut back)?;
    // Lab both endpoints of the round trip (normalized out; un-normalize
    // for real ΔE76 values).
    let mut lab_w = vec![0f32; count * 3];
    let mut lab_back = vec![0f32; count * 3];
    t_w_lab.apply_f32(&w, &mut lab_w)?;
    t_w_lab.apply_f32(&back, &mut lab_back)?;

    let mut data = Vec::with_capacity(count);
    for i in 0..count {
        let la = lab_from_normalized([
            f64::from(lab_w[i * 3]),
            f64::from(lab_w[i * 3 + 1]),
            f64::from(lab_w[i * 3 + 2]),
        ]);
        let lb = lab_from_normalized([
            f64::from(lab_back[i * 3]),
            f64::from(lab_back[i * 3 + 1]),
            f64::from(lab_back[i * 3 + 2]),
        ]);
        let flag = if delta_e76(la, lb) > GAMUT_WARNING_THRESHOLD_DE76 {
            1.0
        } else {
            0.0
        };
        data.push([d[i * 3], d[i * 3 + 1], d[i * 3 + 2], flag]);
    }
    Ok(ProofLut {
        size: n as u32,
        data,
    })
}

/// The §5.6 CPU reference of the shader's flag test: "`> 0.5` recovers a
/// crisp region" from the trilinearly interpolated flag.
#[inline]
pub fn gamut_flagged(sampled_alpha: f32) -> bool {
    sampled_alpha > 0.5
}

/// Trilinear CPU reference sampler over the rgba proof LUT (§5.4
/// "Display and soft-proof LUTs: trilinear"; §5.6: "identical on the CPU
/// reference"). `enc` is encoded working RGB in `[0,1]` (clamped, like
/// the WGSL). The flag interpolates like the color.
pub fn sample_lut3d_rgba(lut: &ProofLut, enc: [f32; 3]) -> [f32; 4] {
    let n = lut.size as usize;
    let idx = |r: usize, g: usize, b: usize| -> [f32; 4] { lut.data[(b * n + g) * n + r] };
    let scale = (n - 1) as f32;
    let p: Vec<(usize, usize, f32)> = enc
        .iter()
        .map(|&c| {
            let x = c.clamp(0.0, 1.0) * scale;
            let lo = (x.floor() as usize).min(n - 1);
            let hi = (lo + 1).min(n - 1);
            (lo, hi, x - lo as f32)
        })
        .collect();
    let (r0, r1, fr) = p[0];
    let (g0, g1, fg) = p[1];
    let (b0, b1, fb) = p[2];
    let mut out = [0f32; 4];
    for (k, o) in out.iter_mut().enumerate() {
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let c00 = lerp(idx(r0, g0, b0)[k], idx(r1, g0, b0)[k], fr);
        let c10 = lerp(idx(r0, g1, b0)[k], idx(r1, g1, b0)[k], fr);
        let c01 = lerp(idx(r0, g0, b1)[k], idx(r1, g0, b1)[k], fr);
        let c11 = lerp(idx(r0, g1, b1)[k], idx(r1, g1, b1)[k], fr);
        *o = lerp(lerp(c00, c10, fg), lerp(c01, c11, fg), fb);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::BuiltinSpace;

    fn srgb_bytes() -> Vec<u8> {
        lcms2::Profile::new_srgb().icc().unwrap()
    }

    /// Display-LUT bake (no proof): 33³ rgba-free lattice; sRGB→sRGB-bytes
    /// display is appearance-identity, so every lattice point maps near
    /// itself; determinism: two bakes are byte-identical (§5.8 test 4,
    /// per-machine half).
    #[test]
    fn display_lut_bakes_and_is_deterministic() {
        let mut cms = Cms::new();
        let display = cms.open_profile(srgb_bytes(), ColorModel::Rgb).unwrap();
        let working = cms.builtin(BuiltinSpace::Srgb);
        let a = bake_display_lut(
            &mut cms,
            working,
            display,
            Intent::RelativeColorimetric,
            true,
        )
        .unwrap();
        assert_eq!(a.size as usize, LUT_SIZE);
        assert_eq!(a.data.len(), LUT_SIZE * LUT_SIZE * LUT_SIZE);
        // Appearance identity: encoded in ≈ encoded out.
        let n = LUT_SIZE;
        for (i, px) in a.data.iter().enumerate() {
            let r = (i % n) as f32 / (n - 1) as f32;
            let g = ((i / n) % n) as f32 / (n - 1) as f32;
            let b = (i / (n * n)) as f32 / (n - 1) as f32;
            assert!((px[0] - r).abs() < 2e-3, "lattice {i}: {px:?} vs r={r}");
            assert!((px[1] - g).abs() < 2e-3);
            assert!((px[2] - b).abs() < 2e-3);
        }
        let b = bake_display_lut(
            &mut cms,
            working,
            display,
            Intent::RelativeColorimetric,
            true,
        )
        .unwrap();
        for (x, y) in a.data.iter().zip(b.data.iter()) {
            for c in 0..3 {
                assert_eq!(x[c].to_bits(), y[c].to_bits());
            }
        }
    }

    /// Proof chain, in-gamut case: proofing sRGB working against a
    /// synthesized Display-P3 profile — sRGB ⊂ P3, so the round trip is
    /// tight and the gamut flag must be 0 everywhere; rgb ≈ identity
    /// (P3 covers sRGB; display is sRGB again).
    #[test]
    fn proof_lut_in_gamut_has_no_flags() {
        let p3_bytes = crate::icc::test_builtin_icc_bytes(BuiltinSpace::DisplayP3);
        let mut cms = Cms::new();
        let display = cms.open_profile(srgb_bytes(), ColorModel::Rgb).unwrap();
        let proof_profile = cms.open_profile(p3_bytes, ColorModel::Rgb).unwrap();
        let working = cms.builtin(BuiltinSpace::Srgb);
        let sp = SoftProof {
            proof_profile,
            intent: Intent::RelativeColorimetric,
            bpc: true,
            simulate_paper: false,
            gamut_warning: true,
        };
        let lut = bake_proof_lut(&mut cms, working, display, &sp).unwrap();
        assert_eq!(lut.data.len(), LUT_SIZE * LUT_SIZE * LUT_SIZE);
        let flagged = lut.data.iter().filter(|px| px[3] > 0.5).count();
        assert_eq!(flagged, 0, "sRGB inside P3 must not flag");
        // And the chain is appearance-preserving for in-gamut colors.
        let n = LUT_SIZE;
        for (i, px) in lut.data.iter().enumerate() {
            let r = (i % n) as f32 / (n - 1) as f32;
            assert!((px[0] - r).abs() < 5e-3, "lattice {i}: {px:?}");
        }
    }

    /// Proof chain, out-of-gamut case: proofing ProPhoto working against
    /// an sRGB proof profile — much of the ProPhoto lattice is outside
    /// sRGB, so a substantial region must flag; neutrals near the axis
    /// must NOT flag (they are in every gamut).
    #[test]
    fn proof_lut_flags_out_of_gamut_regions() {
        let mut cms = Cms::new();
        let display = cms.open_profile(srgb_bytes(), ColorModel::Rgb).unwrap();
        let proof_profile = cms.open_profile(srgb_bytes(), ColorModel::Rgb).unwrap();
        let working = cms.builtin(BuiltinSpace::ProPhotoRgb);
        let sp = SoftProof {
            proof_profile,
            intent: Intent::RelativeColorimetric,
            bpc: false,
            simulate_paper: false,
            gamut_warning: true,
        };
        let lut = bake_proof_lut(&mut cms, working, display, &sp).unwrap();
        let n = LUT_SIZE;
        let flagged = lut.data.iter().filter(|px| px[3] > 0.5).count();
        assert!(
            flagged > lut.data.len() / 10,
            "ProPhoto vs sRGB flagged only {flagged}/{}",
            lut.data.len()
        );
        // The neutral axis stays unflagged (white/black/grays are in
        // both gamuts; relative colorimetric maps white to white).
        for i in 0..n {
            let px = lut.data[(i * n + i) * n + i];
            assert!(px[3] < 0.5, "neutral lattice point {i} flagged: {px:?}");
        }
    }

    /// The trilinear CPU sampler: exact on lattice points, interpolating
    /// between them, flag crisp via the > 0.5 rule.
    #[test]
    fn trilinear_sampler_reference() {
        // A tiny synthetic 2³ LUT: identity rgb, flag set only in the
        // r=1 plane.
        let mut data = Vec::new();
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    data.push([r as f32, g as f32, b as f32, r as f32]);
                }
            }
        }
        let lut = ProofLut { size: 2, data };
        // Lattice-exact.
        let s = sample_lut3d_rgba(&lut, [1.0, 0.0, 1.0]);
        assert_eq!(s, [1.0, 0.0, 1.0, 1.0]);
        // Midpoint interpolates all channels including the flag.
        let m = sample_lut3d_rgba(&lut, [0.5, 0.5, 0.5]);
        assert_eq!(m, [0.5, 0.5, 0.5, 0.5]);
        assert!(!gamut_flagged(m[3]), "0.5 is not > 0.5 — crisp edge rule");
        assert!(gamut_flagged(sample_lut3d_rgba(&lut, [0.8, 0.5, 0.5])[3]));
        // Out-of-range input clamps like the WGSL.
        let c = sample_lut3d_rgba(&lut, [1.5, -0.5, 0.5]);
        assert_eq!(c[0], 1.0);
        assert_eq!(c[1], 0.0);
    }

    /// Gray/CMYK/Lab working spaces refuse the v1 RGB-lattice bake with a
    /// structured error (module-docs posture), never a wrong LUT.
    #[test]
    fn non_rgb_working_refuses_cleanly() {
        let mut cms = Cms::new();
        let display = cms.open_profile(srgb_bytes(), ColorModel::Rgb).unwrap();
        let gray = cms.builtin(BuiltinSpace::SGray);
        assert!(matches!(
            bake_display_lut(&mut cms, gray, display, Intent::RelativeColorimetric, true),
            Err(ColorError::TransformBuild(_))
        ));
    }
}
