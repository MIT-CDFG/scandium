// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §5.2 (LittleCMS integration — the CMM this module wraps;
//! profile validation step 2's deep parse and step 5's lazy transform
//! buildability; thread safety: one `cmsContext` per engine instance,
//! never the global context; `cmsFLAGS_NOCACHE` on every transform;
//! `cmsFLAGS_COPY_ALPHA` so alpha is never color-managed; the
//! [NORMATIVE] pixel-format table, F32 row, incl. the `K(1,4)` CmykA
//! descriptor expression; the [NORMATIVE] normalized-storage ↔ real-value
//! Lab marshalling `L = 100·v₀`, `a = 255·v₁ − 128`, `b = 255·v₂ − 128`),
//! §5.5 (gray working spaces synthesized at runtime — no third-party
//! profile bytes shipped; CMYK MUST go through an ICC profile), §6.2
//! (mode-conversion CMS chains — the CP-1.0 `doc.mode_convert` machinery
//! rides these pairs), Appendix B (the constants the builtin-profile
//! synthesis derives from); deps record `deps/littlecms2.yaml` (lcms2
//! 6.1.1 / lcms2-sys 4.0.7, MIT; the `-sys` crate is the DD-① sanctioned
//! unsafe location — THIS crate stays `#![forbid(unsafe_code)]`,
//! everything here goes through the safe `lcms2` wrapper).
//!
//! ## Determinism (D-48) — flag elections of record
//! * **`Flags::NO_CACHE` on every transform** (§5.2 [NORMATIVE]): the
//!   per-transform one-pixel cache is a data race under concurrent
//!   `cmsDoTransform` AND a batch-split history dependence — with it,
//!   output bytes could depend on how tiles were chunked. `NO_CACHE` also
//!   makes the transform `Sync` in the safe wrapper's type system.
//! * **`Flags::COPY_ALPHA`** on alpha-bearing layouts: the `EXTRA`
//!   channel copies through untouched — alpha is never color-managed.
//! * **`Flags::BLACKPOINT_COMPENSATION`** exactly when the key's `bpc` is
//!   set (a colorimetry *parameter*, keyed in the cache — not an
//!   optimization election).
//! * **No optimization/precision flags are elected** (`NO_OPTIMIZE`,
//!   `HIGHRES_PRECALC`, `LOWRES_PRECALC`, CLUT forcing all unset): the
//!   lcms2 defaults are used, so output bytes are a pure function of
//!   (profile bytes, formats, intent, flags) — bit-stable per machine and
//!   build, which is the D-48 replay surface. Electing any flag that
//!   changes rounding requires a fresh determinism note in the lane log
//!   FIRST.
//!
//! ## Value marshalling (CP-1.0 slice)
//! Buffers everywhere in Appname are **normalized f32** (`0..=1` per
//! channel, ES §2.2/§6.1). lcms2's float formats are not: float **Lab**
//! is real-valued (`L* 0..100`, `a*/b* −128..127`) and float **CMYK** is
//! ink percentage (`0..100`). This module scales at the CMS boundary —
//! the §5.2 Lab formulas verbatim, and `×100 / ÷100` for CMYK — in a
//! fixed scalar order (one multiply(+add) per value, no reassociation),
//! alpha untouched. Gray/RGB float formats are already `0..1`; no scaling.
//!
//! ## Context lifetime (safe-Rust keep-alive)
//! The engine owns its `cmsContext` behind `Arc<Mutex<ThreadContext>>`.
//! Every built transform holds a clone of that `Arc`, so the context
//! provably outlives every transform without any `unsafe` — the `Mutex`
//! wrapper is what makes the (Send-only) `ThreadContext` shareable inside
//! `Arc`. Builds serialize on the lock (§5.2: build under a lock, apply
//! lock-free); `cmsDoTransform` runs without the lock because `NO_CACHE`
//! transforms are `Sync` by the wrapper's own rules.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use lcms2::{
    CIExyY, CIExyYTRIPLE, Flags, PixelFormat as LcmsFormat, Profile, ThreadContext, ToneCurve,
    Transform as LcmsTransform,
};

use crate::error::ColorError;
use crate::profile::{ProfileEntry, ProfileSource};
use crate::space::BuiltinSpace;
use crate::types::{ColorModel, Intent};

/// §5.2 table cells without named lcms2 constants, built from the
/// normative descriptor expressions (the spec pins the bit-packing as
/// version-stable; `EXTRA_SH(1)` = `1 << 7`):
/// * `G(1,4)` GrayA/F32 = `GRAY_FLT | EXTRA_SH(1)`
/// * `K(1,4)` CmykA/F32 = `CMYK_FLT | EXTRA_SH(1)`
const GRAYA_FLT: LcmsFormat = LcmsFormat(LcmsFormat::GRAY_FLT.0 | (1 << 7));
const CMYKA_FLT: LcmsFormat = LcmsFormat(LcmsFormat::CMYK_FLT.0 | (1 << 7));

/// A thread-safe lcms2 transform over interleaved f32 pixel arrays.
type Tr<I, O> = LcmsTransform<I, O, ThreadContext, lcms2::DisallowCache>;

/// How a model's normalized-storage values map onto the lcms2 float
/// encoding (module docs "Value marshalling").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Marshal {
    /// Gray/RGB float formats: already normalized. Identity.
    None,
    /// CMYK float is ink percentage: `×100` in, `÷100` out (color
    /// channels only; alpha untouched).
    Cmyk,
    /// Lab float is real-valued; §5.2 formulas: in `L = 100·v₀`,
    /// `a = 255·v₁ − 128`, `b = 255·v₂ − 128`; out the inverses.
    Lab,
}

fn marshal_of(model: ColorModel) -> Marshal {
    match model.without_alpha() {
        ColorModel::Cmyk => Marshal::Cmyk,
        ColorModel::Lab => Marshal::Lab,
        _ => Marshal::None,
    }
}

/// Scale one pixel's color channels storage → lcms encoding, in place.
/// Fixed scalar order; alpha (channels beyond `color_ch`) untouched.
fn scale_in(px: &mut [f32], color_ch: usize, m: Marshal) {
    match m {
        Marshal::None => {}
        Marshal::Cmyk => {
            for v in &mut px[..color_ch] {
                *v *= 100.0;
            }
        }
        Marshal::Lab => {
            px[0] *= 100.0;
            px[1] = 255.0 * px[1] - 128.0;
            px[2] = 255.0 * px[2] - 128.0;
        }
    }
}

/// Scale one pixel's color channels lcms encoding → storage, in place.
fn scale_out(px: &mut [f32], color_ch: usize, m: Marshal) {
    match m {
        Marshal::None => {}
        Marshal::Cmyk => {
            for v in &mut px[..color_ch] {
                *v /= 100.0;
            }
        }
        Marshal::Lab => {
            px[0] /= 100.0;
            px[1] = (px[1] + 128.0) / 255.0;
            px[2] = (px[2] + 128.0) / 255.0;
        }
    }
}

/// Everything a typed pair build needs beyond the two profile handles
/// (bundled so the macro-generated constructor keeps a civil arity).
#[derive(Clone, Copy)]
struct PairSpec {
    fmt_in: LcmsFormat,
    fmt_out: LcmsFormat,
    ch_in: usize,
    ch_out: usize,
    intent: lcms2::Intent,
    flags: Flags<lcms2::DisallowCache>,
}

/// The typed (arity-in, arity-out) transform pairs. Since the CP-1.0
/// slice every §5.2 F32-row model pair is buildable, so the enum covers
/// the full arity grid {1,2,3,4,5}²; the lcms format (which knows the
/// model) travels separately — the typed wrapper only pins pixel widths.
macro_rules! icc_pairs {
    ($( $variant:ident : $ci:literal -> $co:literal ),+ $(,)?) => {
        enum IccPairTr {
            $( $variant(Tr<[f32; $ci], [f32; $co]>), )+
        }

        impl IccPairTr {
            fn build(
                guard: &ThreadContext,
                src_p: &Profile<ThreadContext>,
                dst_p: &Profile<ThreadContext>,
                spec: PairSpec,
            ) -> Result<IccPairTr, ColorError> {
                let build_err =
                    || ColorError::TransformBuild("cmsCreateTransform refused the profile pair");
                Ok(match (spec.ch_in, spec.ch_out) {
                    $(
                        ($ci, $co) => IccPairTr::$variant(
                            Tr::new_flags_context(
                                guard, src_p, spec.fmt_in, dst_p, spec.fmt_out,
                                spec.intent, spec.flags,
                            )
                            .map_err(|_| build_err())?,
                        ),
                    )+
                    _ => {
                        return Err(ColorError::TransformBuild(
                            "pixel arity outside the §5.2 F32 table",
                        ))
                    }
                })
            }

            fn apply(&self, src: &[f32], dst: &mut [f32]) {
                match self {
                    $(
                        IccPairTr::$variant(t) => {
                            let (s, _) = src.as_chunks::<$ci>();
                            let (d, _) = dst.as_chunks_mut::<$co>();
                            t.transform_pixels(s, d);
                        }
                    )+
                }
            }
        }
    };
}

icc_pairs! {
    A1x1: 1 -> 1, A1x2: 1 -> 2, A1x3: 1 -> 3, A1x4: 1 -> 4, A1x5: 1 -> 5,
    A2x1: 2 -> 1, A2x2: 2 -> 2, A2x3: 2 -> 3, A2x4: 2 -> 4, A2x5: 2 -> 5,
    A3x1: 3 -> 1, A3x2: 3 -> 2, A3x3: 3 -> 3, A3x4: 3 -> 4, A3x5: 3 -> 5,
    A4x1: 4 -> 1, A4x2: 4 -> 2, A4x3: 4 -> 3, A4x4: 4 -> 4, A4x5: 4 -> 5,
    A5x1: 5 -> 1, A5x2: 5 -> 2, A5x3: 5 -> 3, A5x4: 5 -> 4, A5x5: 5 -> 5,
}

/// A built ICC transform: the typed lcms2 handle, the boundary marshals,
/// plus the context keep-alive (see module docs). `Send + Sync` by
/// construction — `NO_CACHE` transforms are `Sync`, the context rides in
/// `Arc<Mutex<_>>`.
pub(crate) struct IccTransform {
    pair: IccPairTr,
    model_in: ColorModel,
    model_out: ColorModel,
    marshal_in: Marshal,
    marshal_out: Marshal,
    /// Keep-alive only — never locked on the apply path.
    _ctx: Arc<Mutex<ThreadContext>>,
}

impl fmt::Debug for IccTransform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "IccTransform({:?}->{:?}, NOCACHE)",
            self.model_in, self.model_out
        )
    }
}

impl IccTransform {
    /// Apply over interleaved normalized f32 pixels (lengths already
    /// validated by the caller — `Transform::apply_f32` — to be whole
    /// pixels on both sides). Lab/CMYK endpoints scale through scratch
    /// buffers at the boundary (module docs); one allocation per call
    /// (per tile batch), never per pixel.
    pub(crate) fn apply_f32(&self, src: &[f32], dst: &mut [f32]) {
        let ch_in = self.model_in.channels();
        let ch_out = self.model_out.channels();
        match (self.marshal_in, self.marshal_out) {
            (Marshal::None, Marshal::None) => self.pair.apply(src, dst),
            (m_in, m_out) => {
                let color_in = self.model_in.color_channels();
                let color_out = self.model_out.color_channels();
                let mut scratch = src.to_vec();
                for px in scratch.chunks_exact_mut(ch_in) {
                    scale_in(px, color_in, m_in);
                }
                self.pair.apply(&scratch, dst);
                for px in dst.chunks_exact_mut(ch_out) {
                    scale_out(px, color_out, m_out);
                }
            }
        }
    }
}

/// The lcms2-backed engine: one `cmsContext` per instance (§5.2 — the
/// global context is never used).
pub(crate) struct IccEngine {
    ctx: Arc<Mutex<ThreadContext>>,
}

impl fmt::Debug for IccEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IccEngine(cmsContext per engine instance)")
    }
}

impl IccEngine {
    pub(crate) fn new() -> IccEngine {
        IccEngine {
            ctx: Arc::new(Mutex::new(ThreadContext::new())),
        }
    }

    fn lock(&self) -> MutexGuard<'_, ThreadContext> {
        // Poison recovery keeps this panic-free (§1.6): the context is a
        // handle; a panicked builder thread leaves it structurally intact.
        self.ctx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// §5.2 validation step 2, deep half: the real CMM parse. Header-level
    /// checks (steps 1, 3, 4) already ran in the registry; this catches
    /// truncated tag DATA, bad tag types, and everything else header
    /// inspection cannot see. Step 5 (transform buildability) stays lazy
    /// at [`IccEngine::build`], per the spec.
    pub(crate) fn deep_parse(&self, bytes: &[u8]) -> Result<(), ColorError> {
        let guard = self.lock();
        Profile::<ThreadContext>::new_icc_context(&*guard, bytes)
            .map(|_| ())
            .map_err(|_| ColorError::ProfileMalformed("CMM parse rejected the profile"))
    }

    /// Build the lcms2 transform for any §5.2 F32-row model pair (the
    /// caller has already gated layouts and profile/model space matches).
    pub(crate) fn build(
        &self,
        src: &ProfileEntry,
        dst: &ProfileEntry,
        model_in: ColorModel,
        model_out: ColorModel,
        intent: Intent,
        bpc: bool,
    ) -> Result<IccTransform, ColorError> {
        let guard = self.lock();
        let src_p = profile_for(&guard, src)?;
        let dst_p = profile_for(&guard, dst)?;
        let fmt_in = lcms_format(model_in);
        let fmt_out = lcms_format(model_out);
        let li = lcms_intent(intent);
        // Flag elections of record (module docs): NOCACHE always;
        // COPY_ALPHA on alpha-bearing layouts; BPC per key.
        let mut flags = Flags::NO_CACHE;
        if model_in.has_alpha() {
            flags = flags | Flags::COPY_ALPHA;
        }
        if bpc {
            flags = flags | Flags::BLACKPOINT_COMPENSATION;
        }
        let pair = IccPairTr::build(
            &guard,
            &src_p,
            &dst_p,
            PairSpec {
                fmt_in,
                fmt_out,
                ch_in: model_in.channels(),
                ch_out: model_out.channels(),
                intent: li,
                flags,
            },
        )?;
        Ok(IccTransform {
            pair,
            model_in,
            model_out,
            marshal_in: marshal_of(model_in),
            marshal_out: marshal_of(model_out),
            _ctx: Arc::clone(&self.ctx),
        })
    }
}

/// Resolve a registry entry to an lcms2 profile handle. Builtins are
/// synthesized at runtime from the spec's own constants (§5.5 posture: no
/// third-party profile bytes shipped); ICC entries reopen their bytes
/// (already deep-parsed at ingress; a failure here is still a clean
/// error, §1.6).
fn profile_for(
    ctx: &ThreadContext,
    entry: &ProfileEntry,
) -> Result<Profile<ThreadContext>, ColorError> {
    match &entry.source {
        ProfileSource::IccBytes(bytes) => Profile::new_icc_context(ctx, bytes)
            .map_err(|_| ColorError::ProfileMalformed("CMM parse rejected the profile")),
        ProfileSource::Builtin(space) => synthesize_builtin(ctx, *space),
        ProfileSource::BuiltinLab => synthesize_lab(ctx),
    }
}

/// Runtime synthesis of the builtin CIE Lab (D50) profile — the §5.2
/// `builtin()` Lab entry, via `cmsCreateLab4Profile` (v4; the §5.2 rule
/// "Lab crossings use float formats only" keeps us off the 16-bit v2 Lab
/// encoding pitfalls entirely).
fn synthesize_lab(ctx: &ThreadContext) -> Result<Profile<ThreadContext>, ColorError> {
    let d50 = crate::chromatic::WhitePoint::D50.xyz();
    let white = xy_from_xyz(d50[0], d50[1], d50[2]);
    Profile::new_lab4_context(ctx, &white)
        .map_err(|_| ColorError::TransformBuild("builtin Lab profile synthesis failed"))
}

/// Runtime synthesis of the builtin working-space profiles (§5.5).
fn synthesize_builtin(
    ctx: &ThreadContext,
    space: BuiltinSpace,
) -> Result<Profile<ThreadContext>, ColorError> {
    let err = ColorError::TransformBuild("builtin profile synthesis failed");
    match space {
        // lcms2's own sRGB constructor is the §5.2 `builtin(sRGB)` door.
        BuiltinSpace::Srgb => Ok(Profile::new_srgb_context(ctx)),
        BuiltinSpace::DisplayP3 | BuiltinSpace::ProPhotoRgb => {
            // Chromaticities derived mechanically from the Appendix B
            // matrix columns (x = X/(X+Y+Z) per column; white = row sums)
            // — no constants beyond the spec's own tables.
            let m = space
                .rgb_to_xyz_native()
                .ok_or(ColorError::TransformBuild("gray space has no RGB matrix"))?;
            let primaries = CIExyYTRIPLE {
                Red: xy_of_column(&m.m, 0),
                Green: xy_of_column(&m.m, 1),
                Blue: xy_of_column(&m.m, 2),
            };
            let white = xy_of_white(&m.m);
            let trc = tone_curve(space)?;
            Profile::new_rgb_context(ctx, &white, &primaries, &[&trc, &trc, &trc]).map_err(|_| err)
        }
        BuiltinSpace::GrayGamma22 | BuiltinSpace::SGray => {
            // §5.5: synthesized via cmsCreateGrayProfile, D50 white.
            let d50 = crate::chromatic::WhitePoint::D50.xyz();
            let white = xy_from_xyz(d50[0], d50[1], d50[2]);
            let trc = tone_curve(space)?;
            Profile::new_gray_context(ctx, &white, &trc).map_err(|_| err)
        }
    }
}

/// lcms2 tone curve matching `TransferFn` for the builtin space, via
/// parametric curves (exact analytic forms, not tables).
fn tone_curve(space: BuiltinSpace) -> Result<ToneCurve, ColorError> {
    let err = ColorError::TransformBuild("tone-curve construction failed");
    match space {
        // IEC 61966-2-1 piecewise pair (§5.5 constants), lcms2 parametric
        // type 4: Y = (aX+b)^γ for X ≥ d, else cX.
        BuiltinSpace::Srgb | BuiltinSpace::DisplayP3 | BuiltinSpace::SGray => {
            ToneCurve::new_parametric(4, &[2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045])
                .map_err(|_| err)
        }
        // Pure power 2.2 (§5.5 gray default).
        BuiltinSpace::GrayGamma22 => Ok(ToneCurve::new(2.2)),
        // ROMM curve as parametric type 4 (see transfer.rs provenance
        // note / OQ-COLOR-1): Y = X^1.8 for X ≥ 16/512, else X/16.
        BuiltinSpace::ProPhotoRgb => {
            ToneCurve::new_parametric(4, &[1.8, 1.0, 0.0, 1.0 / 16.0, 16.0 / 512.0])
                .map_err(|_| err)
        }
    }
}

fn xy_from_xyz(x: f64, y: f64, z: f64) -> CIExyY {
    let sum = x + y + z;
    CIExyY {
        x: x / sum,
        y: y / sum,
        Y: 1.0,
    }
}

fn xy_of_column(m: &[[f64; 3]; 3], col: usize) -> CIExyY {
    xy_from_xyz(m[0][col], m[1][col], m[2][col])
}

fn xy_of_white(m: &[[f64; 3]; 3]) -> CIExyY {
    let x = m[0][0] + m[0][1] + m[0][2];
    let y = m[1][0] + m[1][1] + m[1][2];
    let z = m[2][0] + m[2][1] + m[2][2];
    xy_from_xyz(x, y, z)
}

/// The §5.2 pixel-format mapping, F32 row — now total over the eight
/// models (the CP-1.0 slice; CmykA is the normative `K(1,4)` expression,
/// GrayA the `G(1,4)` one).
fn lcms_format(model: ColorModel) -> LcmsFormat {
    match model {
        ColorModel::Gray => LcmsFormat::GRAY_FLT,
        ColorModel::GrayA => GRAYA_FLT,
        ColorModel::Rgb => LcmsFormat::RGB_FLT,
        ColorModel::Rgba => LcmsFormat::RGBA_FLT,
        ColorModel::Cmyk => LcmsFormat::CMYK_FLT,
        ColorModel::CmykA => CMYKA_FLT,
        ColorModel::Lab => LcmsFormat::Lab_FLT,
        ColorModel::LabA => LcmsFormat::LabA_FLT,
    }
}

fn lcms_intent(intent: Intent) -> lcms2::Intent {
    match intent {
        Intent::Perceptual => lcms2::Intent::Perceptual,
        Intent::RelativeColorimetric => lcms2::Intent::RelativeColorimetric,
        Intent::Saturation => lcms2::Intent::Saturation,
        Intent::AbsoluteColorimetric => lcms2::Intent::AbsoluteColorimetric,
    }
}

/// Test support (crate-internal): serialize a builtin space's synthesized
/// profile to ICC bytes — real profile bytes for the transform tests,
/// generated in-process (no third-party profile bytes in the repo).
#[cfg(test)]
pub(crate) fn test_builtin_icc_bytes(space: BuiltinSpace) -> Vec<u8> {
    let engine = IccEngine::new();
    let guard = engine.lock();
    let p = synthesize_builtin(&guard, space).unwrap();
    p.icc().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The constructed descriptor expressions must satisfy lcms2's own
    /// arithmetic: G(1,4) = 2 × 4 B/px; K(1,4) = 5 × 4 B/px.
    #[test]
    fn constructed_descriptors_are_consistent() {
        assert_eq!(GRAYA_FLT.bytes_per_pixel(), 8);
        assert_eq!(CMYKA_FLT.bytes_per_pixel(), 20);
        assert_eq!(LcmsFormat::GRAY_FLT.bytes_per_pixel(), 4);
        assert_eq!(LcmsFormat::RGB_FLT.bytes_per_pixel(), 12);
        assert_eq!(LcmsFormat::RGBA_FLT.bytes_per_pixel(), 16);
        assert_eq!(LcmsFormat::CMYK_FLT.bytes_per_pixel(), 16);
        assert_eq!(LcmsFormat::Lab_FLT.bytes_per_pixel(), 12);
        assert_eq!(LcmsFormat::LabA_FLT.bytes_per_pixel(), 16);
    }

    /// Builtin synthesis succeeds for all five spaces and yields profiles
    /// whose serialized bytes the real CMM re-opens, and whose headers
    /// pass this crate's §5.2 header validation (both validation layers
    /// agree on our own synthesized output).
    #[test]
    fn builtin_synthesis_round_trips_through_the_cmm() {
        let engine = IccEngine::new();
        let guard = engine.lock();
        for space in BuiltinSpace::ALL {
            let p = synthesize_builtin(&guard, space).unwrap();
            let bytes = p.icc().unwrap();
            let reopened = Profile::<ThreadContext>::new_icc_context(&*guard, &bytes);
            assert!(
                reopened.is_ok(),
                "{space:?}: serialized builtin failed to reopen"
            );
            let header = crate::profile::parse_icc_header(&bytes).unwrap();
            assert!(header.class.usable_as_working(), "{space:?}: {header:?}");
        }
    }

    /// The builtin Lab profile synthesizes, and its data color space is
    /// Lab (the §5.2 builtin() Lab entry).
    #[test]
    fn lab_synthesis_is_a_lab_profile() {
        let engine = IccEngine::new();
        let guard = engine.lock();
        let p = synthesize_lab(&guard).unwrap();
        assert_eq!(p.color_space(), lcms2::ColorSpaceSignature::LabData);
    }

    /// The Lab/CMYK boundary marshals invert each other (fixed-order
    /// scalar chain; §5.2 formulas + the ink-percentage rule).
    #[test]
    fn marshalling_round_trips() {
        let mut lab = [0.25f32, 0.5, 0.75, 0.5];
        scale_in(&mut lab, 3, Marshal::Lab);
        assert_eq!(lab[0], 25.0);
        assert_eq!(lab[1], 255.0 * 0.5 - 128.0);
        assert_eq!(lab[3], 0.5, "alpha untouched");
        scale_out(&mut lab, 3, Marshal::Lab);
        assert!((lab[0] - 0.25).abs() < 1e-6);
        assert!((lab[1] - 0.5).abs() < 1e-6);
        assert!((lab[2] - 0.75).abs() < 1e-6);

        let mut cmyk = [0.1f32, 0.2, 0.3, 0.4, 1.0];
        scale_in(&mut cmyk, 4, Marshal::Cmyk);
        assert_eq!(cmyk[0], 10.0);
        assert_eq!(cmyk[4], 1.0, "alpha untouched");
        scale_out(&mut cmyk, 4, Marshal::Cmyk);
        assert!((cmyk[0] - 0.1).abs() < 1e-6);
        assert!((cmyk[3] - 0.4).abs() < 1e-6);
    }
}
