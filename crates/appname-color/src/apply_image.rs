// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §14.4 (Calculations / Apply Image — "The math [NORMATIVE]"
//! block, implemented to the letter: the `S′/S″` conditioning chain, the
//! two legacy arithmetic modes that exist ONLY here, the mask arm
//! `m(p) = mask × selection`, `preserveTransparency` as the §11.1
//! transparency-lock clamp), §10.3 `channel.apply_image` /
//! `channel.calculations` rows (param ranges: `opacity 0..=1`,
//! `scale 1..=2`, `offset −1..=1`), §4.5 (the blend kernels `B(T, S)` the
//! non-legacy modes call — injected, see below), §11.1 (TRANSPARENCY lock
//! semantics: "constrains paint compositing to existing coverage"),
//! §13.1 (the active selection as a coverage gate); DD area ⑲.
//!
//! ## One math library (the §14.4 bit-identity clause)
//! [NORMATIVE §14.4] `channel.apply_image` with mode `multiply`,
//! opacity 1, no scale/offset, no mask MUST be bit-identical to
//! compositing the same source over the target as a Multiply layer —
//! "one math library, tested once". The §4.5 kernels are that one
//! library and live with the compositor (`appname-render`, O-RENDER's
//! fence). This module therefore does NOT re-implement §4.5: the kernel
//! is **injected**, and this module owns exactly the §14.4-specific
//! math around it (conditioning, opacity/mask lerp, the legacy `add` /
//! `subtract` arithmetic, the transparency clamp). Two injection arms
//! match render's kernel routing:
//! * separable modes → [`apply_blend_plane`], kernel
//!   `B(T, S) : fn(f32, f32) -> f32`, one component plane at a time;
//! * non-separable (Hue/Saturation/Color/Luminosity) and whole-pixel
//!   (Darker/Lighter Color) modes → [`apply_blend_planes3`], kernel
//!   `B(T_rgb, S_rgb) : fn([f32; 3], [f32; 3]) -> [f32; 3]` over the
//!   three component planes together;
//! * `dissolve` → typed-reserved, [`dissolve_reserved`] (it is coverage
//!   noise needing document coordinates, which the §14.4 math block
//!   does not carry — divergence recorded to fold-in).
//!
//! The §14.4 `S′/S″` conditioning is exported
//! ([`ApplyShape::source_prime`] / [`ApplyShape::condition_source`]) so
//! no other crate ever re-derives it — the same one-math-library
//! discipline, pointed at this module. Core wires render's
//! CPU-reference kernels in at the command layer; the §34 bit-identity
//! test runs there, against the single kernel source.
//!
//! ## Evaluation domain
//! Per §14.4, everything evaluates per pixel **in working space (linear
//! f32)**; single-channel sources "ride as Gray" — the unit of work is
//! the planar `f32` slice. Multi-component targets (a layer under
//! `channel:"rgb"`) run the separable arm per component, or the triple
//! arm once, with the same coverage planes; `S″` conditions per channel
//! in both arms. All arithmetic is fixed-order scalar f32 —
//! deterministic bytes for the D-48 replay surface.

use crate::error::ColorError;

/// The §14.4 conditioning + application parameters (the command params
/// minus source/target plumbing, which is core's resolution job).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ApplyShape {
    /// `invertSource`: `S′ = 1 − S` before everything else.
    pub invert_source: bool,
    /// `opacity ∈ [0, 1]` (§10.3 row).
    pub opacity: f32,
    /// `scale ∈ [1, 2]` (§10.3 row; neutral at 1).
    pub scale: f32,
    /// `offset ∈ [−1, 1]` normalized (§14.4: the UI shows ×255 at u8).
    pub offset: f32,
}

impl Default for ApplyShape {
    fn default() -> ApplyShape {
        ApplyShape {
            invert_source: false,
            opacity: 1.0,
            scale: 1.0,
            offset: 0.0,
        }
    }
}

impl ApplyShape {
    /// Range-check against the §10.3 rows. Envelope validation is core's
    /// stage-3 job; this is the engine-side refusal so the math can never
    /// run on out-of-contract values (this crate does not clamp silently
    /// — non-finite/NaN policy is the validator's, §40.1).
    pub fn validate(&self) -> Result<(), ColorError> {
        if !(self.opacity >= 0.0 && self.opacity <= 1.0) {
            return Err(ColorError::ParamOutOfRange("opacity outside 0..=1"));
        }
        if !(self.scale >= 1.0 && self.scale <= 2.0) {
            return Err(ColorError::ParamOutOfRange("scale outside 1..=2"));
        }
        if !(self.offset >= -1.0 && self.offset <= 1.0) {
            return Err(ColorError::ParamOutOfRange("offset outside -1..=1"));
        }
        Ok(())
    }

    /// `S′` — invert only (the legacy arithmetic modes consume this
    /// directly: they "skip the `S″` step (no double application)").
    ///
    /// Public: the §14.4 conditioning belongs to THIS module alone (the
    /// one-math-library discipline) — consumers building further arms
    /// call these, never re-derive them.
    #[inline]
    pub fn source_prime(&self, s: f32) -> f32 {
        if self.invert_source {
            1.0 - s
        } else {
            s
        }
    }

    /// `S″ = clamp(S′ / scale + offset, 0, 1)` — the pre-blend
    /// conditioning for §4.5-kernel modes; neutral at scale=1, offset=0.
    /// Applied **per channel** — a multi-component source conditions
    /// each component independently (the §14.4 math block is stated on
    /// values; nothing couples channels before the kernel).
    #[inline]
    pub fn condition_source(&self, s: f32) -> f32 {
        (self.source_prime(s) / self.scale + self.offset).clamp(0.0, 1.0)
    }
}

/// The §14.4 coverage arm: `m(p)` = optional mask coverage (invertible)
/// × the active selection (§13.1) — "both honored, absent ⇒ 1" — and,
/// when `preserveTransparency:true` on a layer target, the target's
/// existing alpha as the §11.1 transparency-lock clamp ("constrains
/// paint compositing to existing coverage"): pass the alpha plane as
/// [`CoverageArm::target_alpha`] exactly in that case (core resolves the
/// flag; a channel target never passes one).
#[derive(Clone, Copy, Default)]
pub struct CoverageArm<'a> {
    /// The `mask` source's coverage plane, if a mask was given.
    pub mask: Option<&'a [f32]>,
    /// `mask.invert` — applied to the mask plane only.
    pub invert_mask: bool,
    /// The active selection's coverage plane, if a selection is active.
    pub selection: Option<&'a [f32]>,
    /// The target layer's existing alpha, iff `preserveTransparency`.
    pub target_alpha: Option<&'a [f32]>,
}

impl CoverageArm<'_> {
    /// All present planes must match the working plane's length.
    fn validate(&self, len: usize) -> Result<(), ColorError> {
        for plane in [self.mask, self.selection, self.target_alpha]
            .into_iter()
            .flatten()
        {
            if plane.len() != len {
                return Err(ColorError::ParamOutOfRange(
                    "coverage plane length mismatch",
                ));
            }
        }
        Ok(())
    }

    /// `m(p)` at index `i` (fixed multiply order: mask, selection, alpha).
    #[inline]
    fn at(&self, i: usize) -> f32 {
        let mut m = match self.mask {
            Some(mk) => {
                if self.invert_mask {
                    1.0 - mk[i]
                } else {
                    mk[i]
                }
            }
            None => 1.0,
        };
        if let Some(sel) = self.selection {
            m *= sel[i];
        }
        if let Some(a) = self.target_alpha {
            m *= a[i];
        }
        m
    }
}

/// `lerp(t, v, w) = t + (v − t)·w` — the §14.4 application lerp, fixed
/// operation order, with the endpoints EXACT: at `w = 1` the result is
/// `v` bit-for-bit (the §14.4 bit-identity clause — opacity 1, no mask
/// MUST equal the bare §4.5 kernel; `t + (v − t)` re-rounds and would
/// miss by an ulp) and at `w = 0` it is `t` bit-for-bit (an
/// unmasked-out pixel is untouched).
#[inline]
fn lerp(t: f32, v: f32, w: f32) -> f32 {
    if w == 1.0 {
        v
    } else {
        t + (v - t) * w
    }
}

fn check_lengths(target: &[f32], source: &[f32], cov: &CoverageArm<'_>) -> Result<(), ColorError> {
    if target.len() != source.len() {
        return Err(ColorError::ParamOutOfRange(
            "target/source plane length mismatch",
        ));
    }
    cov.validate(target.len())
}

/// §14.4, the §4.5-kernel modes:
/// `R = lerp(T, B(T, S″), opacity · m(p))` with
/// `S″ = clamp(S′ / scale + offset, 0, 1)`, `S′ = invert ? 1 − S : S`.
///
/// `kernel` is the §4.5 separable blend kernel `B(T, S)` for the
/// command's `mode` (injected — module docs "One math library").
pub fn apply_blend_plane<F: Fn(f32, f32) -> f32>(
    kernel: F,
    target: &mut [f32],
    source: &[f32],
    shape: &ApplyShape,
    cov: &CoverageArm<'_>,
) -> Result<(), ColorError> {
    shape.validate()?;
    check_lengths(target, source, cov)?;
    for (i, (t, s)) in target.iter_mut().zip(source.iter()).enumerate() {
        let s2 = shape.condition_source(*s);
        let blended = kernel(*t, s2);
        *t = lerp(*t, blended, shape.opacity * cov.at(i));
    }
    Ok(())
}

/// §14.4, the §4.5 NON-SEPARABLE and WHOLE-PIXEL modes (Hue,
/// Saturation, Color, Luminosity via the Appendix-A `Lum/Sat/ClipColor/
/// SetLum/SetSat` helpers; Darker/Lighter Color's whole-pixel luminance
/// picks): the kernel is a **pixel-triple** function
/// `B(T_rgb, S_rgb) → R_rgb`, injected exactly like the separable arm
/// (render's routing types these `fn([f32;3],[f32;3])->[f32;3]`; one
/// math library, still render's).
///
/// §14.4 semantics unchanged: `S″` conditioning applies **per channel**
/// ([`ApplyShape::condition_source`] — the conditioning stays this
/// module's, exported through this arm, never duplicated); the coverage
/// weight `opacity · m(p)` is computed ONCE per pixel and the
/// application lerp runs per channel with that shared weight (the
/// whole-pixel property survives at full weight: w = 1 hands back the
/// kernel's triple bit-for-bit).
///
/// `targets`/`sources` are the three component planes (R, G, B order —
/// the §14.4 layer target under `channel:"rgb"`), equal lengths;
/// coverage planes as in the separable arm.
pub fn apply_blend_planes3<F: Fn([f32; 3], [f32; 3]) -> [f32; 3]>(
    kernel: F,
    targets: [&mut [f32]; 3],
    sources: [&[f32]; 3],
    shape: &ApplyShape,
    cov: &CoverageArm<'_>,
) -> Result<(), ColorError> {
    shape.validate()?;
    let len = targets[0].len();
    if targets.iter().any(|t| t.len() != len) || sources.iter().any(|s| s.len() != len) {
        return Err(ColorError::ParamOutOfRange(
            "component plane length mismatch",
        ));
    }
    cov.validate(len)?;
    let [t0, t1, t2] = targets;
    for i in 0..len {
        let t = [t0[i], t1[i], t2[i]];
        let s2 = [
            shape.condition_source(sources[0][i]),
            shape.condition_source(sources[1][i]),
            shape.condition_source(sources[2][i]),
        ];
        let b = kernel(t, s2);
        let w = shape.opacity * cov.at(i);
        t0[i] = lerp(t[0], b[0], w);
        t1[i] = lerp(t[1], b[1], w);
        t2[i] = lerp(t[2], b[2], w);
    }
    Ok(())
}

/// Dissolve's routing, named [ruled, O-INT D1]: `dissolve` is in the
/// contract's blend enum (§10.3: "§4.5 names + add, subtract") but the
/// §14.4 math block defines `R` purely per-value — `lerp(T, B(T, S″),
/// opacity·m)` — with no document-coordinate context, while §4.5
/// Dissolve [NORMATIVE] is BN64 coordinate-hashed coverage noise, not a
/// value kernel `B(T, S)`. Dissolve-in-apply-image is therefore
/// **typed-reserved**: core's arm returns this error for
/// `blend:"dissolve"`; the other §4.5 modes all execute (separable arm,
/// or [`apply_blend_planes3`] for the 6 triple modes). The
/// contract-vs-§14.4 divergence is recorded to the fold-in list in the
/// lane log — if the fold-in pins dissolve semantics (a coordinate-fed
/// arm), it lands here, next to the conditioning it will share.
pub fn dissolve_reserved() -> ColorError {
    ColorError::Reserved(
        "blend 'dissolve' in apply-image/calculations: §14.4 defines no coordinate-context \
         semantics for it (contract divergence recorded to fold-in)",
    )
}

/// §14.4 legacy `add` — exists only in apply-image/calculations; scale
/// and offset apply INSIDE the arithmetic (the classic definition), and
/// the `S″` step is skipped:
/// `R = lerp(T, clamp((T + S′) / scale + offset, 0, 1), opacity · m(p))`.
pub fn apply_add_plane(
    target: &mut [f32],
    source: &[f32],
    shape: &ApplyShape,
    cov: &CoverageArm<'_>,
) -> Result<(), ColorError> {
    shape.validate()?;
    check_lengths(target, source, cov)?;
    for (i, (t, s)) in target.iter_mut().zip(source.iter()).enumerate() {
        let v = ((*t + shape.source_prime(*s)) / shape.scale + shape.offset).clamp(0.0, 1.0);
        *t = lerp(*t, v, shape.opacity * cov.at(i));
    }
    Ok(())
}

/// §14.4 legacy `subtract` (same shape as `add`, minus):
/// `R = lerp(T, clamp((T − S′) / scale + offset, 0, 1), opacity · m(p))`.
pub fn apply_subtract_plane(
    target: &mut [f32],
    source: &[f32],
    shape: &ApplyShape,
    cov: &CoverageArm<'_>,
) -> Result<(), ColorError> {
    shape.validate()?;
    check_lengths(target, source, cov)?;
    for (i, (t, s)) in target.iter_mut().zip(source.iter()).enumerate() {
        let v = ((*t - shape.source_prime(*s)) / shape.scale + shape.offset).clamp(0.0, 1.0);
        *t = lerp(*t, v, shape.opacity * cov.at(i));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand oracle kernels for the tests ONLY (the production kernel is
    /// render's §4.5 CPU reference, injected by core — module docs).
    fn multiply(t: f32, s: f32) -> f32 {
        t * s
    }
    fn screen(t: f32, s: f32) -> f32 {
        t + s - t * s
    }

    fn seeded(n: usize, mut state: u64) -> Vec<f32> {
        (0..n)
            .map(|_| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (((state >> 11) as f64) / ((1u64 << 53) as f64)) as f32
            })
            .collect()
    }

    /// §14.4 bit-identity clause, plane-level half: mode multiply,
    /// opacity 1, neutral scale/offset, no mask ⇒ R == B(T, S) exactly
    /// (the lerp at weight 1 and the neutral conditioning must vanish
    /// bit-for-bit for in-range values).
    #[test]
    fn neutral_params_reduce_to_the_bare_kernel() {
        let t0 = seeded(512, 0xA11CE);
        let s = seeded(512, 0xB0B);
        let mut t = t0.clone();
        apply_blend_plane(
            multiply,
            &mut t,
            &s,
            &ApplyShape::default(),
            &CoverageArm::default(),
        )
        .unwrap();
        for i in 0..t.len() {
            assert_eq!(t[i].to_bits(), (t0[i] * s[i]).to_bits(), "px {i}");
        }
    }

    /// Worked §14.4 conditioning oracle: invert, scale 2, offset −0.25,
    /// opacity 0.5, no mask. S=0.2 → S′=0.8 → S″=clamp(0.4−0.25)=0.15;
    /// B=T·S″; R = T + (B−T)·0.5.
    #[test]
    fn conditioning_chain_matches_the_hand_computation() {
        let shape = ApplyShape {
            invert_source: true,
            opacity: 0.5,
            scale: 2.0,
            offset: -0.25,
        };
        let mut t = [0.6f32];
        apply_blend_plane(multiply, &mut t, &[0.2], &shape, &CoverageArm::default()).unwrap();
        let s2 = (0.8f32 / 2.0 - 0.25).clamp(0.0, 1.0);
        let want = 0.6 + (0.6 * s2 - 0.6) * 0.5;
        assert_eq!(t[0].to_bits(), want.to_bits());
    }

    /// Legacy add/subtract: scale/offset INSIDE the arithmetic, no S″
    /// (no double application). Hand oracles per the §14.4 formulas.
    #[test]
    fn legacy_add_subtract_formulas() {
        let shape = ApplyShape {
            invert_source: false,
            opacity: 1.0,
            scale: 2.0,
            offset: 0.1,
        };
        let mut t = [0.5f32];
        apply_add_plane(&mut t, &[0.3], &shape, &CoverageArm::default()).unwrap();
        let want_add = ((0.5f32 + 0.3) / 2.0 + 0.1).clamp(0.0, 1.0);
        assert_eq!(t[0].to_bits(), want_add.to_bits());

        let mut t = [0.5f32];
        apply_subtract_plane(&mut t, &[0.3], &shape, &CoverageArm::default()).unwrap();
        let want_sub = ((0.5f32 - 0.3) / 2.0 + 0.1).clamp(0.0, 1.0);
        assert_eq!(t[0].to_bits(), want_sub.to_bits());

        // Subtract clamps at 0: T=0.1, S=0.9, neutral scale/offset.
        let mut t = [0.1f32];
        apply_subtract_plane(
            &mut t,
            &[0.9],
            &ApplyShape::default(),
            &CoverageArm::default(),
        )
        .unwrap();
        assert_eq!(t[0], 0.0);

        // Add clamps at 1.
        let mut t = [0.7f32];
        apply_add_plane(
            &mut t,
            &[0.9],
            &ApplyShape::default(),
            &CoverageArm::default(),
        )
        .unwrap();
        assert_eq!(t[0], 1.0);
    }

    /// The mask arm: m(p) = mask (invertible) × selection; both honored,
    /// absent ⇒ 1; m = 0 leaves the target bit-untouched.
    #[test]
    fn mask_and_selection_gate_coverage() {
        let t0 = [0.25f32, 0.25, 0.25, 0.25];
        let s = [1.0f32, 1.0, 1.0, 1.0];
        let mask = [0.0f32, 1.0, 0.5, 1.0];
        let sel = [1.0f32, 0.0, 0.5, 1.0];
        let mut t = t0;
        apply_blend_plane(
            screen,
            &mut t,
            &s,
            &ApplyShape::default(),
            &CoverageArm {
                mask: Some(&mask),
                invert_mask: false,
                selection: Some(&sel),
                target_alpha: None,
            },
        )
        .unwrap();
        // B(0.25, 1) = 1 for screen; R = lerp(0.25, 1, m).
        for (i, m) in [0.0f32, 0.0, 0.25, 1.0].iter().enumerate() {
            let want = 0.25 + (1.0 - 0.25) * m;
            assert_eq!(t[i].to_bits(), want.to_bits(), "px {i}");
        }
        // Inverted mask flips the first arm.
        let mut t = t0;
        apply_blend_plane(
            screen,
            &mut t,
            &s,
            &ApplyShape::default(),
            &CoverageArm {
                mask: Some(&mask),
                invert_mask: true,
                selection: None,
                target_alpha: None,
            },
        )
        .unwrap();
        assert_eq!(t[0], 1.0, "inverted 0-mask is full coverage");
        assert_eq!(t[1].to_bits(), t0[1].to_bits(), "inverted 1-mask is 0");
    }

    /// preserveTransparency = the §11.1 transparency-lock clamp: writes
    /// constrained to existing coverage — zero-alpha pixels bit-untouched,
    /// partial alpha scales the write.
    #[test]
    fn preserve_transparency_clamps_to_existing_alpha() {
        let t0 = [0.2f32, 0.2, 0.2];
        let alpha = [0.0f32, 0.5, 1.0];
        let mut t = t0;
        apply_blend_plane(
            screen,
            &mut t,
            &[1.0, 1.0, 1.0],
            &ApplyShape::default(),
            &CoverageArm {
                mask: None,
                invert_mask: false,
                selection: None,
                target_alpha: Some(&alpha),
            },
        )
        .unwrap();
        assert_eq!(t[0].to_bits(), t0[0].to_bits(), "a=0: untouched");
        let half: f32 = 0.2 + (1.0 - 0.2) * 0.5;
        assert_eq!(t[1].to_bits(), half.to_bits());
        assert_eq!(t[2], 1.0);
    }

    /// Param ranges are the §10.3 rows; out-of-contract values refuse
    /// (structured, never silently clamped here).
    #[test]
    fn out_of_range_params_refuse() {
        for shape in [
            ApplyShape {
                opacity: 1.5,
                ..ApplyShape::default()
            },
            ApplyShape {
                scale: 0.5,
                ..ApplyShape::default()
            },
            ApplyShape {
                scale: 2.5,
                ..ApplyShape::default()
            },
            ApplyShape {
                offset: -1.5,
                ..ApplyShape::default()
            },
            ApplyShape {
                opacity: f32::NAN,
                ..ApplyShape::default()
            },
        ] {
            let mut t = [0.5f32];
            assert!(matches!(
                apply_blend_plane(multiply, &mut t, &[0.5], &shape, &CoverageArm::default()),
                Err(ColorError::ParamOutOfRange(_))
            ));
        }
        // Length mismatches refuse too.
        let mut t = [0.5f32; 2];
        assert!(apply_add_plane(
            &mut t,
            &[0.5],
            &ApplyShape::default(),
            &CoverageArm::default()
        )
        .is_err());
    }

    // ---- §4.5 non-separable / whole-pixel oracles (test-only; the
    // production kernels are render's — the Appendix-A helper math is
    // restated here purely as the hand oracle) ----

    fn lum(c: [f32; 3]) -> f32 {
        0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
    }
    fn clip_color(c: [f32; 3]) -> [f32; 3] {
        let l = lum(c);
        let n = c[0].min(c[1]).min(c[2]);
        let x = c[0].max(c[1]).max(c[2]);
        let mut out = c;
        if n < 0.0 {
            for v in &mut out {
                *v = l + (*v - l) * l / (l - n);
            }
        }
        if x > 1.0 {
            for v in &mut out {
                *v = l + (*v - l) * (1.0 - l) / (x - l);
            }
        }
        out
    }
    fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
        let d = l - lum(c);
        clip_color([c[0] + d, c[1] + d, c[2] + d])
    }
    /// §4.5 Luminosity: `set_lum(cb, lum(cs))`.
    fn luminosity(cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
        set_lum(cb, lum(cs))
    }
    /// §4.5 Darker Color: whole-pixel pick, tie → backdrop.
    fn darker_color(cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
        if lum(cb) <= lum(cs) {
            cb
        } else {
            cs
        }
    }

    /// Triple arm, non-separable oracle: Luminosity at neutral params
    /// reduces to the bare kernel bit-for-bit (w = 1 endpoint), and the
    /// result carries the source's luminance while keeping the target's
    /// chroma direction — checked against the Appendix-A hand math.
    #[test]
    fn triple_arm_luminosity_oracle() {
        let t = [[0.8f32, 0.1], [0.2f32, 0.5], [0.1f32, 0.9]]; // 2 px, planar RGB
        let s = [[0.3f32, 0.6], [0.3f32, 0.6], [0.3f32, 0.6]]; // neutral gray source
        let mut t0 = t[0];
        let mut t1 = t[1];
        let mut t2 = t[2];
        apply_blend_planes3(
            luminosity,
            [&mut t0, &mut t1, &mut t2],
            [&s[0], &s[1], &s[2]],
            &ApplyShape::default(),
            &CoverageArm::default(),
        )
        .unwrap();
        for px in 0..2 {
            let want = luminosity(
                [t[0][px], t[1][px], t[2][px]],
                [s[0][px], s[1][px], s[2][px]],
            );
            assert_eq!(t0[px].to_bits(), want[0].to_bits(), "px {px}");
            assert_eq!(t1[px].to_bits(), want[1].to_bits(), "px {px}");
            assert_eq!(t2[px].to_bits(), want[2].to_bits(), "px {px}");
            // And the §4.5 property: result luminance == source luminance.
            let got_lum = lum([t0[px], t1[px], t2[px]]);
            let want_lum = lum([s[0][px], s[1][px], s[2][px]]);
            assert!((got_lum - want_lum).abs() < 1e-5, "px {px}");
        }
    }

    /// Triple arm, whole-pixel mode: Darker Color returns ONE input's
    /// triple untouched (never a per-channel mix), tie → backdrop; at
    /// w = 1 the pick survives bit-for-bit.
    #[test]
    fn triple_arm_whole_pixel_pick() {
        // Target darker at px 0, source darker at px 1, tie at px 2
        // (same luminance, different channels: lum-weights make
        // (0.59, 0, 0.3·?) … use scaled primaries with equal lum).
        let t = [
            [0.1f32, 0.9, 0.59], // R plane
            [0.1f32, 0.9, 0.00], // G plane
            [0.1f32, 0.9, 0.00], // B plane
        ];
        // px2 target = (0.59,0,0): lum = 0.3·0.59 = 0.177
        // px2 source = (0,0.3,0):  lum = 0.59·0.3 = 0.177 → tie
        let s = [[0.5f32, 0.5, 0.0], [0.5f32, 0.5, 0.3], [0.5f32, 0.5, 0.0]];
        let mut t0 = t[0];
        let mut t1 = t[1];
        let mut t2 = t[2];
        apply_blend_planes3(
            darker_color,
            [&mut t0, &mut t1, &mut t2],
            [&s[0], &s[1], &s[2]],
            &ApplyShape::default(),
            &CoverageArm::default(),
        )
        .unwrap();
        // px 0: target (0.1,0.1,0.1) is darker — untouched whole pixel.
        assert_eq!([t0[0], t1[0], t2[0]], [0.1, 0.1, 0.1]);
        // px 1: source (0.5,0.5,0.5) darker than (0.9,0.9,0.9).
        assert_eq!([t0[1], t1[1], t2[1]], [0.5, 0.5, 0.5]);
        // px 2: tie resolves to the BACKDROP (§4.5 [NORMATIVE]).
        assert_eq!([t0[2], t1[2], t2[2]], [0.59, 0.0, 0.0]);
    }

    /// Per-channel S″ on a multi-component target: with invert/scale/
    /// offset live, each source component conditions independently
    /// through the SAME exported conditioning the separable arm uses —
    /// verified against a manual per-channel S″ + kernel + lerp chain,
    /// and against three separable-arm runs with a per-channel kernel
    /// (arm equivalence for separable math).
    #[test]
    fn triple_arm_conditions_per_channel() {
        let shape = ApplyShape {
            invert_source: true,
            opacity: 0.6,
            scale: 1.5,
            offset: -0.2,
        };
        let mask = [0.9f32, 0.4];
        let cov = CoverageArm {
            mask: Some(&mask),
            invert_mask: false,
            selection: None,
            target_alpha: None,
        };
        let t = [[0.7f32, 0.2], [0.4f32, 0.8], [0.9f32, 0.35]];
        let s = [[0.15f32, 0.8], [0.5f32, 0.05], [0.95f32, 0.6]];
        // Triple arm with a multiply-per-channel kernel (separable math
        // expressed as a triple kernel).
        let mul3 = |cb: [f32; 3], cs: [f32; 3]| [cb[0] * cs[0], cb[1] * cs[1], cb[2] * cs[2]];
        let mut a0 = t[0];
        let mut a1 = t[1];
        let mut a2 = t[2];
        apply_blend_planes3(
            mul3,
            [&mut a0, &mut a1, &mut a2],
            [&s[0], &s[1], &s[2]],
            &shape,
            &cov,
        )
        .unwrap();
        // Manual per-channel chain via the exported conditioning.
        for px in 0..2 {
            let w = shape.opacity * mask[px];
            for (ch, (tp, sp)) in t.iter().zip(s.iter()).enumerate() {
                let s2 = shape.condition_source(sp[px]);
                let want = tp[px] + (tp[px] * s2 - tp[px]) * w;
                let got = [&a0, &a1, &a2][ch][px];
                assert_eq!(got.to_bits(), want.to_bits(), "px {px} ch {ch}");
            }
        }
        // Arm equivalence: three separable-arm runs produce the same bytes.
        let mut b0 = t[0];
        let mut b1 = t[1];
        let mut b2 = t[2];
        for (tb, sb) in [(&mut b0, &s[0]), (&mut b1, &s[1]), (&mut b2, &s[2])] {
            apply_blend_plane(|cb, cs| cb * cs, tb, sb, &shape, &cov).unwrap();
        }
        for ((x, y), (u, v)) in a0.iter().zip(b0.iter()).zip(a1.iter().zip(b1.iter())) {
            assert_eq!(x.to_bits(), y.to_bits());
            assert_eq!(u.to_bits(), v.to_bits());
        }
        for (x, y) in a2.iter().zip(b2.iter()) {
            assert_eq!(x.to_bits(), y.to_bits());
        }
    }

    /// Triple-arm coverage endpoints: a masked-out pixel is untouched
    /// bit-for-bit across all three planes; length mismatches refuse.
    #[test]
    fn triple_arm_gates_and_lengths() {
        let t = [[0.3f32, 0.3], [0.6f32, 0.6], [0.9f32, 0.9]];
        let s = [[1.0f32, 1.0], [1.0f32, 1.0], [1.0f32, 1.0]];
        let mask = [0.0f32, 1.0];
        let mut t0 = t[0];
        let mut t1 = t[1];
        let mut t2 = t[2];
        apply_blend_planes3(
            luminosity,
            [&mut t0, &mut t1, &mut t2],
            [&s[0], &s[1], &s[2]],
            &ApplyShape::default(),
            &CoverageArm {
                mask: Some(&mask),
                invert_mask: false,
                selection: None,
                target_alpha: None,
            },
        )
        .unwrap();
        assert_eq!(t0[0].to_bits(), t[0][0].to_bits());
        assert_eq!(t1[0].to_bits(), t[1][0].to_bits());
        assert_eq!(t2[0].to_bits(), t[2][0].to_bits());
        // px 1 at full coverage: white source lifts luminance to 1.
        assert!((lum([t0[1], t1[1], t2[1]]) - 1.0).abs() < 1e-5);
        // Length mismatch across component planes refuses.
        let mut short = [0.5f32];
        let mut x1 = [0.5f32, 0.5];
        let mut x2 = [0.5f32, 0.5];
        assert!(apply_blend_planes3(
            luminosity,
            [&mut short, &mut x1, &mut x2],
            [&s[0], &s[1], &s[2]],
            &ApplyShape::default(),
            &CoverageArm::default(),
        )
        .is_err());
    }

    /// Dissolve's routing is typed-reserved, named, and distinct from
    /// out-of-range (the wire value IS contract-valid).
    #[test]
    fn dissolve_is_typed_reserved() {
        let e = dissolve_reserved();
        assert!(matches!(e, ColorError::Reserved(_)));
        assert_eq!(e.wire_code(), None);
    }

    /// Determinism: two runs over seeded planes are bit-identical.
    #[test]
    fn apply_is_bit_deterministic() {
        let t0 = seeded(1000, 0xD48);
        let s = seeded(1000, 0x144);
        let mask = seeded(1000, 0x3a5);
        let shape = ApplyShape {
            invert_source: true,
            opacity: 0.7,
            scale: 1.5,
            offset: 0.05,
        };
        let run = || {
            let mut t = t0.clone();
            apply_blend_plane(
                screen,
                &mut t,
                &s,
                &shape,
                &CoverageArm {
                    mask: Some(&mask),
                    invert_mask: false,
                    selection: None,
                    target_alpha: None,
                },
            )
            .unwrap();
            t
        };
        let a = run();
        let b = run();
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.to_bits(), y.to_bits());
        }
    }
}
