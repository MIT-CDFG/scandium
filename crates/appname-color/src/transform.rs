// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §5.2 (the `Cms`/`Transform` shape; premultiplied handling —
//! "the CMS never sees premultiplied values"; alpha is never
//! color-managed; thread safety: transforms shared read-only across
//! workers), §5.3 (assign ≠ convert: this module is the CONVERT machinery;
//! assign is a retag with zero pixel change and lives at core's command
//! layer), §5.4 (transform cache keyed `(src, dst, proof, intent, bpc)` —
//! extended per plan row with the marshalling depth; shaper/matrix fast
//! path `decode-TRC → 3×3 → encode-TRC` with `M = M_dst⁻¹ · M_adapt ·
//! M_src` precomputed on CPU), §5.5 (gray↔RGB via the neutral axis / Y),
//! §5.8 test 2 (identity transform bit-exactness — null-transform
//! detection).
//!
//! ## Determinism (D-48) — the contract this module keeps
//! Same inputs → same output bytes, always:
//! * The pixel path is a fixed f64 chain (`decode → 3×3 → encode`)
//!   narrowed to f32 once, at the end. No fused-multiply-add, no
//!   reassociation, no data-dependent branching beyond the TRC piecewise
//!   breaks (which are value-deterministic).
//! * The cache is a `BTreeMap` keyed on [`TransformKey`] (total order —
//!   no hash-seed or insertion-order dependence anywhere).
//! * The identity path is a byte copy — bit-exact by construction.
//! * lcms2 flag plan, recorded now for the backend slice: transforms will
//!   be created with `cmsFLAGS_NOCACHE` (§5.2 thread-safety rule: the
//!   per-transform one-pixel cache is a data race AND a
//!   history-dependence — with it, output bytes could depend on batch
//!   split) and `cmsFLAGS_COPY_ALPHA` (alpha passes through untouched);
//!   no `cmsFLAGS_OPTIMIZE`-class speedups that change rounding may be
//!   elected without a lane-log determinism note.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::ColorError;
use crate::icc::{IccEngine, IccTransform};
use crate::mat3::Mat3;
use crate::profile::{parse_icc_header, ProfileRegistry, ProfileSource, ProfileUid};
use crate::space::{rgb_to_rgb_matrix, BuiltinSpace};
use crate::transfer::TransferFn;
use crate::types::{ColorModel, Depth, Intent, PixelFormat};

/// §5.4 cache key, extended with the marshalling layouts (plan §2.3 row:
/// "keyed on profile pair + intent + depth"). `proof` is `None` until the
/// soft-proof slice (§5.6) — carried now so the key shape is stable.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TransformKey {
    pub src: ProfileUid,
    pub dst: ProfileUid,
    pub proof: Option<ProfileUid>,
    pub intent: Intent,
    pub bpc: bool,
    pub fmt_in: PixelFormat,
    pub fmt_out: PixelFormat,
}

/// The compiled per-pixel operation.
///
/// Intent semantics on the pure paths, documented per §5.4/§5.5: the
/// builtin spaces are matrix/TRC class — they carry no perceptual or
/// saturation tables, their black points are zero (BPC is the identity),
/// and their ICC media white is the PCS white after adaptation (absolute
/// == relative). All four intents therefore compile to the same
/// colorimetric math HERE, exactly as lcms2 treats matrix-only profiles;
/// `intent`/`bpc` stay in the key so profile-driven paths (which do
/// differ) can never alias a pure entry.
#[derive(Debug)]
enum TransformOp {
    /// Null transform (§5.8 test 2): src and dst resolve to the same
    /// profile and layout — a byte copy, bit-exact.
    Identity,
    /// RGB → RGB shaper/matrix fast path (§5.4).
    RgbMatrix {
        decode: TransferFn,
        m: Mat3,
        encode: TransferFn,
    },
    /// Gray → RGB: decode gray TRC → linear Y → equal linear RGB on the
    /// working neutral axis (§5.5) → encode dst TRC.
    GrayToRgb {
        decode: TransferFn,
        encode: TransferFn,
    },
    /// RGB → Gray: decode → linear RGB → Y via the D50-adapted luminance
    /// row (§5.5 "takes the Y of XYZ") → encode gray TRC.
    RgbToGray {
        decode: TransferFn,
        y_row: [f64; 3],
        encode: TransferFn,
    },
    /// Gray → Gray: decode src TRC → linear Y → encode dst TRC.
    GrayToGray {
        decode: TransferFn,
        encode: TransferFn,
    },
    /// Profile-driven path through the real CMM (ES §5.2; `icc.rs` —
    /// lcms2 with the recorded flag elections `NOCACHE`/`COPY_ALPHA`).
    Icc(IccTransform),
}

/// A compiled color transform (ES §5.2 `Transform`). Immutable and
/// `Send + Sync`: safe to share across rayon workers; apply takes `&self`.
#[derive(Debug)]
pub struct Transform {
    key: TransformKey,
    op: TransformOp,
}

impl Transform {
    pub fn key(&self) -> &TransformKey {
        &self.key
    }

    /// Whether this is the §5.8-test-2 null transform (byte copy).
    pub fn is_identity(&self) -> bool {
        matches!(self.op, TransformOp::Identity)
    }

    /// Apply over interleaved f32 pixels in the key's layouts.
    ///
    /// [NORMATIVE preconditions, ES §5.2] `src`/`dst` carry STRAIGHT
    /// (un-premultiplied) values — the tile marshaller un-premultiplies
    /// before and re-premultiplies after (guarding `a == 0`, §6.4); that
    /// marshalling lives with the buffer owner (core), not here. Alpha, if
    /// the layout has it, copies through untouched (never color-managed).
    ///
    /// Slice-1 scope: both layouts must be `Depth::F32` (working precision,
    /// §6.1) and share the same alpha-bearing shape; integer/half
    /// marshalling attaches with the lcms2 backend.
    pub fn apply_f32(&self, src: &[f32], dst: &mut [f32]) -> Result<(), ColorError> {
        let ch_in = self.key.fmt_in.channels();
        let ch_out = self.key.fmt_out.channels();
        if !src.len().is_multiple_of(ch_in) || src.len() / ch_in != dst.len() / ch_out.max(1) {
            return Err(ColorError::ProfileMalformed("pixel buffer length mismatch"));
        }
        match &self.op {
            TransformOp::Identity => {
                dst.copy_from_slice(src);
            }
            TransformOp::RgbMatrix { decode, m, encode } => {
                let has_alpha = self.key.fmt_in.model.has_alpha();
                for (s, d) in src.chunks_exact(ch_in).zip(dst.chunks_exact_mut(ch_out)) {
                    let lin = [
                        decode.decode(f64::from(s[0])),
                        decode.decode(f64::from(s[1])),
                        decode.decode(f64::from(s[2])),
                    ];
                    let out = m.mul_vec(lin);
                    d[0] = encode.encode(out[0]) as f32;
                    d[1] = encode.encode(out[1]) as f32;
                    d[2] = encode.encode(out[2]) as f32;
                    if has_alpha {
                        d[3] = s[3]; // alpha untouched (§5.2)
                    }
                }
            }
            TransformOp::GrayToRgb { decode, encode } => {
                let has_alpha = self.key.fmt_in.model.has_alpha();
                for (s, d) in src.chunks_exact(ch_in).zip(dst.chunks_exact_mut(ch_out)) {
                    let y = decode.decode(f64::from(s[0]));
                    let v = encode.encode(y) as f32;
                    d[0] = v;
                    d[1] = v;
                    d[2] = v;
                    if has_alpha {
                        d[3] = s[1];
                    }
                }
            }
            TransformOp::RgbToGray {
                decode,
                y_row,
                encode,
            } => {
                let has_alpha = self.key.fmt_in.model.has_alpha();
                for (s, d) in src.chunks_exact(ch_in).zip(dst.chunks_exact_mut(ch_out)) {
                    let lin = [
                        decode.decode(f64::from(s[0])),
                        decode.decode(f64::from(s[1])),
                        decode.decode(f64::from(s[2])),
                    ];
                    let y = y_row[0] * lin[0] + y_row[1] * lin[1] + y_row[2] * lin[2];
                    d[0] = encode.encode(y) as f32;
                    if has_alpha {
                        d[1] = s[3];
                    }
                }
            }
            TransformOp::GrayToGray { decode, encode } => {
                let has_alpha = self.key.fmt_in.model.has_alpha();
                for (s, d) in src.chunks_exact(ch_in).zip(dst.chunks_exact_mut(ch_out)) {
                    d[0] = encode.encode(decode.decode(f64::from(s[0]))) as f32;
                    if has_alpha {
                        d[1] = s[1];
                    }
                }
            }
            TransformOp::Icc(t) => {
                t.apply_f32(src, dst);
            }
        }
        Ok(())
    }
}

/// The CMM facade (ES §5.2 `Cms` shape). Owns the profile registry, the
/// per-engine `cmsContext` (via [`IccEngine`]), and the §5.4 transform
/// cache. Interior state is `&mut`-gated; the *transforms* it hands out
/// are `Arc` and freely shared across workers (§5.2 thread-safety model:
/// build under a lock, apply lock-free — `NOCACHE` transforms are `Sync`).
#[derive(Debug)]
pub struct Cms {
    registry: ProfileRegistry,
    icc: IccEngine,
    cache: BTreeMap<TransformKey, Arc<Transform>>,
}

impl Default for Cms {
    fn default() -> Cms {
        Cms::new()
    }
}

impl Cms {
    pub fn new() -> Cms {
        Cms {
            registry: ProfileRegistry::new(),
            icc: IccEngine::new(),
            cache: BTreeMap::new(),
        }
    }

    /// The §5.2 profile ingress (`open_profile`): full validation ladder —
    /// steps 1/3/4 + shallow 2 at header level (pure Rust, pre-FFI), the
    /// deep half of step 2 through the real CMM parse, then content-keyed
    /// registration. Step 5 (transform buildability) stays lazy at
    /// [`Cms::transform`], per the spec.
    pub fn open_profile(
        &mut self,
        bytes: Vec<u8>,
        model: ColorModel,
    ) -> Result<ProfileUid, ColorError> {
        // Header ladder first: cheap, and it bounds what reaches the FFI
        // boundary (the §34.2 fuzz surface).
        let _ = parse_icc_header(&bytes)?;
        self.icc.deep_parse(&bytes)?;
        self.registry.register_working_icc(bytes, model)
    }

    pub fn registry(&self) -> &ProfileRegistry {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut ProfileRegistry {
        &mut self.registry
    }

    /// Builtin-space uid convenience (mirrors `ProfileRegistry`).
    pub fn builtin(&self, space: BuiltinSpace) -> ProfileUid {
        self.registry.builtin_uid(space)
    }

    /// The builtin CIE Lab (D50) connection-space profile uid — the §5.2
    /// `builtin()` Lab entry the `doc.mode_convert` Lab arms resolve
    /// against (§6.2 "anything↔Lab is profile-based").
    pub fn builtin_lab(&self) -> ProfileUid {
        self.registry.lab_uid()
    }

    /// Cache size (test/introspection surface for the §5.4 keying tests).
    pub fn cached_transforms(&self) -> usize {
        self.cache.len()
    }

    /// §5.4 invalidation hook: drop every cache entry touching `uid`
    /// (profile change/unload). Deterministic: `BTreeMap::retain` visits in
    /// key order.
    pub fn invalidate_profile(&mut self, uid: ProfileUid) {
        self.cache
            .retain(|k, _| k.src != uid && k.dst != uid && k.proof != Some(uid));
    }

    /// Build (or fetch from the §5.4 cache) the transform for `key`.
    ///
    /// CP-1.0 support matrix:
    /// * src == dst (+ same layouts) → `Identity` (null-transform
    ///   detection, §5.8 test 2);
    /// * builtin ↔ builtin (RGB/Gray) → pure shaper/matrix or Y paths
    ///   (the §5.4 fast-path family; deterministic f64 chain);
    /// * every other §5.2 F32-row pair — ICC-bytes endpoints, the builtin
    ///   Lab connection-space profile, CMYK/Lab layouts (the
    ///   `doc.mode_convert` machinery, §6.2) — → the lcms2 path
    ///   (`icc.rs`, recorded flag elections; normalized↔real Lab/CMYK
    ///   marshalling at the boundary); build failure is the §5.2 step-5
    ///   lazy check → [`ColorError::TransformBuild`]. CMYK/Lab layouts
    ///   REQUIRE a space-matching profile at that endpoint
    ///   ([`ColorError::ProfileSpaceMismatch`] otherwise — §5.5: CMYK
    ///   "MUST go through an ICC profile"; the last-resort naive
    ///   separation lives in `mode_convert`, never here);
    /// * soft-proof keys → [`ColorError::IccBackendPending`] (proof
    ///   chains are baked leg-wise by `proof::bake_proof_lut`).
    pub fn transform(&mut self, key: TransformKey) -> Result<Arc<Transform>, ColorError> {
        if let Some(t) = self.cache.get(&key) {
            return Ok(Arc::clone(t));
        }
        let op = self.compile(&key)?;
        let t = Arc::new(Transform { key, op });
        self.cache.insert(key, Arc::clone(&t));
        Ok(t)
    }

    fn compile(&self, key: &TransformKey) -> Result<TransformOp, ColorError> {
        // Slice-1 layout gate: f32 in, f32 out, matching alpha shape.
        if key.fmt_in.depth != Depth::F32 || key.fmt_out.depth != Depth::F32 {
            let offending = if key.fmt_in.depth != Depth::F32 {
                key.fmt_in
            } else {
                key.fmt_out
            };
            if offending.depth == Depth::F16 {
                return Err(ColorError::DepthPaused(Depth::F16));
            }
            return Err(ColorError::UnsupportedFormat(offending));
        }
        if key.fmt_in.model.has_alpha() != key.fmt_out.model.has_alpha() {
            return Err(ColorError::UnsupportedFormat(key.fmt_out));
        }
        if key.proof.is_some() {
            // A monolithic proof-keyed transform is not a thing this door
            // builds: the §5.6 proof chain is baked leg-by-leg into the
            // rgba proof LUT by `proof::bake_proof_lut` (each leg is an
            // ordinary cached transform). The `proof` slot exists so
            // baked-LUT cache identities can ride the §5.4 key shape.
            return Err(ColorError::IccBackendPending);
        }
        // Null transform first (§5.8 test 2): same profile, same layout.
        if key.src == key.dst && key.fmt_in == key.fmt_out {
            // Both sides must at least resolve.
            let _ = self.registry.get(key.src)?;
            return Ok(TransformOp::Identity);
        }
        let src_color = key.fmt_in.model.without_alpha();
        let dst_color = key.fmt_out.model.without_alpha();
        let src_entry = self.registry.get(key.src)?;
        let dst_entry = self.registry.get(key.dst)?;
        // §5.2 validation 4 at the transform door, both endpoints: the
        // profile's data color space must match the buffer model.
        if !src_entry.data_space.matches_model(src_color)
            || !dst_entry.data_space.matches_model(dst_color)
        {
            return Err(ColorError::ProfileSpaceMismatch);
        }
        let (src_b, dst_b) = match (&src_entry.source, &dst_entry.source) {
            (ProfileSource::Builtin(a), ProfileSource::Builtin(b)) => (*a, *b),
            // Any ICC-bytes endpoint: the profile-driven path through the
            // real CMM (§5.2; flag elections documented in icc.rs).
            _ => {
                return Ok(TransformOp::Icc(self.icc.build(
                    src_entry,
                    dst_entry,
                    key.fmt_in.model,
                    key.fmt_out.model,
                    key.intent,
                    key.bpc,
                )?));
            }
        };
        Ok(match (src_b.is_gray(), dst_b.is_gray()) {
            (false, false) => {
                // rgb_to_rgb_matrix is Some for all non-gray builtins.
                let m = rgb_to_rgb_matrix(src_b, dst_b).ok_or(ColorError::NoMatrixPath)?;
                TransformOp::RgbMatrix {
                    decode: src_b.transfer(),
                    m,
                    encode: dst_b.transfer(),
                }
            }
            (true, false) => TransformOp::GrayToRgb {
                decode: src_b.transfer(),
                encode: dst_b.transfer(),
            },
            (false, true) => TransformOp::RgbToGray {
                decode: src_b.transfer(),
                y_row: src_b.luminance_row_d50().ok_or(ColorError::NoMatrixPath)?,
                encode: dst_b.transfer(),
            },
            (true, true) => TransformOp::GrayToGray {
                decode: src_b.transfer(),
                encode: dst_b.transfer(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::test_support::synthetic_icc;
    use crate::types::Depth;

    fn key(
        cms: &Cms,
        src: BuiltinSpace,
        dst: BuiltinSpace,
        model_in: ColorModel,
        model_out: ColorModel,
    ) -> TransformKey {
        TransformKey {
            src: cms.builtin(src),
            dst: cms.builtin(dst),
            proof: None,
            intent: Intent::RelativeColorimetric,
            bpc: true,
            fmt_in: PixelFormat::new(model_in, Depth::F32),
            fmt_out: PixelFormat::new(model_out, Depth::F32),
        }
    }

    // ---- deterministic test RNG (fixed seed, MMIX LCG constants) ----
    const SEED: u64 = 0xC01_0FEED;
    fn next01(state: &mut u64) -> f32 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (((*state >> 11) as f64) / ((1u64 << 53) as f64)) as f32
    }

    /// §5.8 test 2 groundwork: src == dst is a null transform and the
    /// output is BYTE-identical — including NaN payloads and subnormals,
    /// which survive a copy but not an encode/decode chain.
    #[test]
    fn identity_transform_is_bit_exact() {
        let mut cms = Cms::new();
        let k = key(
            &cms,
            BuiltinSpace::Srgb,
            BuiltinSpace::Srgb,
            ColorModel::Rgba,
            ColorModel::Rgba,
        );
        let t = cms.transform(k).unwrap();
        assert!(t.is_identity());
        let src = vec![
            0.25f32,
            -0.0,
            f32::NAN,
            1.0,
            f32::MIN_POSITIVE / 2.0, // subnormal
            0.999_999_94,
            2.5,
            0.0,
        ];
        let mut dst = vec![0f32; src.len()];
        t.apply_f32(&src, &mut dst).unwrap();
        for (a, b) in src.iter().zip(dst.iter()) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }

    /// sRGB → P3 → sRGB round trip on seeded pixels: sRGB gamut sits
    /// inside P3, so the round trip must return within f32 print
    /// precision; alpha must be untouched bit-for-bit.
    #[test]
    fn srgb_p3_pixel_round_trip() {
        let mut cms = Cms::new();
        let fwd = cms
            .transform(key(
                &cms,
                BuiltinSpace::Srgb,
                BuiltinSpace::DisplayP3,
                ColorModel::Rgba,
                ColorModel::Rgba,
            ))
            .unwrap();
        let back = cms
            .transform(key(
                &cms,
                BuiltinSpace::DisplayP3,
                BuiltinSpace::Srgb,
                ColorModel::Rgba,
                ColorModel::Rgba,
            ))
            .unwrap();
        let mut state = SEED;
        let mut src = Vec::new();
        for _ in 0..512 {
            src.push(next01(&mut state));
            src.push(next01(&mut state));
            src.push(next01(&mut state));
            src.push(next01(&mut state)); // alpha
        }
        let mut mid = vec![0f32; src.len()];
        let mut rt = vec![0f32; src.len()];
        fwd.apply_f32(&src, &mut mid).unwrap();
        back.apply_f32(&mid, &mut rt).unwrap();
        for px in 0..512 {
            for c in 0..3 {
                let a = src[px * 4 + c];
                let b = rt[px * 4 + c];
                assert!((a - b).abs() < 1e-5, "px {px} ch {c}: {a} vs {b}");
            }
            assert_eq!(
                src[px * 4 + 3].to_bits(),
                mid[px * 4 + 3].to_bits(),
                "alpha is never color-managed"
            );
            assert_eq!(src[px * 4 + 3].to_bits(), rt[px * 4 + 3].to_bits());
        }
    }

    /// Applying the same transform to the same bytes twice produces
    /// identical bytes (D-48), and a rebuilt Cms produces the same bytes
    /// again (no hidden state).
    #[test]
    fn apply_is_deterministic_across_instances() {
        let mut state = SEED;
        let src: Vec<f32> = (0..300).map(|_| next01(&mut state)).collect();
        let run = || {
            let mut cms = Cms::new();
            let t = cms
                .transform(key(
                    &cms,
                    BuiltinSpace::Srgb,
                    BuiltinSpace::ProPhotoRgb,
                    ColorModel::Rgb,
                    ColorModel::Rgb,
                ))
                .unwrap();
            let mut dst = vec![0f32; src.len()];
            t.apply_f32(&src, &mut dst).unwrap();
            dst
        };
        let a = run();
        let b = run();
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.to_bits(), y.to_bits());
        }
    }

    /// §5.5 known-value check through the full pixel path: encoded sRGB
    /// white (1,1,1) → ProPhoto must land on encoded white; primary red
    /// through the D50 chain matches the composite matrix directly.
    #[test]
    fn white_maps_to_white_across_builtins() {
        let mut cms = Cms::new();
        for dst in [BuiltinSpace::DisplayP3, BuiltinSpace::ProPhotoRgb] {
            let t = cms
                .transform(key(
                    &cms,
                    BuiltinSpace::Srgb,
                    dst,
                    ColorModel::Rgb,
                    ColorModel::Rgb,
                ))
                .unwrap();
            let src = [1.0f32, 1.0, 1.0];
            let mut out = [0f32; 3];
            t.apply_f32(&src, &mut out).unwrap();
            for c in out {
                assert!((c - 1.0).abs() < 3e-4, "{dst:?}: {out:?}");
            }
        }
    }

    /// Gray → RGB rides the neutral axis (equal channels) and preserves
    /// appearance under a shared TRC; RGB → Gray takes Y (§5.5).
    #[test]
    fn gray_paths_follow_the_neutral_axis() {
        let mut cms = Cms::new();
        // sGray and sRGB share the same TRC, so gray v → RGB (v,v,v).
        let t = cms
            .transform(key(
                &cms,
                BuiltinSpace::SGray,
                BuiltinSpace::Srgb,
                ColorModel::GrayA,
                ColorModel::Rgba,
            ))
            .unwrap();
        let src = [0.5f32, 0.75];
        let mut out = [0f32; 4];
        t.apply_f32(&src, &mut out).unwrap();
        assert!((out[0] - 0.5).abs() < 1e-6);
        assert_eq!(out[0].to_bits(), out[1].to_bits());
        assert_eq!(out[1].to_bits(), out[2].to_bits());
        assert_eq!(out[3].to_bits(), 0.75f32.to_bits());
        // RGB → Gray: pure white → gray white; Y row weights midtones.
        let t2 = cms
            .transform(key(
                &cms,
                BuiltinSpace::Srgb,
                BuiltinSpace::SGray,
                ColorModel::Rgba,
                ColorModel::GrayA,
            ))
            .unwrap();
        let mut g = [0f32; 2];
        t2.apply_f32(&[1.0, 1.0, 1.0, 1.0], &mut g).unwrap();
        assert!((g[0] - 1.0).abs() < 3e-4, "{g:?}");
        assert_eq!(g[1].to_bits(), 1.0f32.to_bits());
        // Green weighs more than blue in Y.
        let mut green = [0f32; 2];
        let mut blue = [0f32; 2];
        t2.apply_f32(&[0.0, 1.0, 0.0, 1.0], &mut green).unwrap();
        t2.apply_f32(&[0.0, 0.0, 1.0, 1.0], &mut blue).unwrap();
        assert!(green[0] > blue[0]);
    }

    /// Gray ↔ Gray TRC re-encode: GrayGamma22 0.5 → sGray keeps linear
    /// luminance (decode 2.2, encode piecewise).
    #[test]
    fn gray_to_gray_reencodes_the_trc() {
        let mut cms = Cms::new();
        let t = cms
            .transform(key(
                &cms,
                BuiltinSpace::GrayGamma22,
                BuiltinSpace::SGray,
                ColorModel::Gray,
                ColorModel::Gray,
            ))
            .unwrap();
        let mut out = [0f32; 1];
        t.apply_f32(&[0.5], &mut out).unwrap();
        let y = TransferFn::Gamma22.decode(0.5);
        let expect = TransferFn::SrgbPiecewise.encode(y) as f32;
        assert_eq!(out[0].to_bits(), expect.to_bits());
    }

    /// §5.4 cache keying: same key → same Arc (hit); every varied
    /// component (intent, bpc, pair, depth) is a distinct entry.
    #[test]
    fn cache_keys_on_pair_intent_bpc_and_depth() {
        let mut cms = Cms::new();
        let base = key(
            &cms,
            BuiltinSpace::Srgb,
            BuiltinSpace::DisplayP3,
            ColorModel::Rgba,
            ColorModel::Rgba,
        );
        let t1 = cms.transform(base).unwrap();
        let t2 = cms.transform(base).unwrap();
        assert!(Arc::ptr_eq(&t1, &t2), "same key must hit the cache");
        assert_eq!(cms.cached_transforms(), 1);
        let mut vary_intent = base;
        vary_intent.intent = Intent::Perceptual;
        cms.transform(vary_intent).unwrap();
        assert_eq!(cms.cached_transforms(), 2);
        let mut vary_bpc = base;
        vary_bpc.bpc = false;
        cms.transform(vary_bpc).unwrap();
        assert_eq!(cms.cached_transforms(), 3);
        let mut vary_pair = base;
        vary_pair.dst = cms.builtin(BuiltinSpace::ProPhotoRgb);
        cms.transform(vary_pair).unwrap();
        assert_eq!(cms.cached_transforms(), 4);
        // Depth variation is part of the key even though non-f32 layouts
        // are refused today — the refusal happens at compile, so assert on
        // the key inequality instead.
        let mut vary_depth = base;
        vary_depth.fmt_in = PixelFormat::new(ColorModel::Rgba, Depth::U16);
        assert_ne!(base, vary_depth);
    }

    /// §5.4 invalidation: dropping a profile removes exactly the entries
    /// touching it.
    #[test]
    fn invalidation_drops_only_matching_entries() {
        let mut cms = Cms::new();
        let srgb_p3 = key(
            &cms,
            BuiltinSpace::Srgb,
            BuiltinSpace::DisplayP3,
            ColorModel::Rgb,
            ColorModel::Rgb,
        );
        let srgb_pp = key(
            &cms,
            BuiltinSpace::Srgb,
            BuiltinSpace::ProPhotoRgb,
            ColorModel::Rgb,
            ColorModel::Rgb,
        );
        let gray_pair = key(
            &cms,
            BuiltinSpace::GrayGamma22,
            BuiltinSpace::SGray,
            ColorModel::Gray,
            ColorModel::Gray,
        );
        cms.transform(srgb_p3).unwrap();
        cms.transform(srgb_pp).unwrap();
        cms.transform(gray_pair).unwrap();
        assert_eq!(cms.cached_transforms(), 3);
        cms.invalidate_profile(cms.builtin(BuiltinSpace::DisplayP3));
        assert_eq!(cms.cached_transforms(), 2);
        cms.invalidate_profile(cms.builtin(BuiltinSpace::Srgb));
        assert_eq!(cms.cached_transforms(), 1);
    }

    /// Structural gates: header-only fake ICC endpoints fail CLEANLY at
    /// the §5.2 step-5 lazy buildability check (or earlier at the CMM
    /// parse — both structured, never a crash), and non-f32 layouts
    /// refuse as unsupported / paused.
    #[test]
    fn build_and_layout_gates() {
        let mut cms = Cms::new();
        // The synthetic fixture is a header + empty tag table — a profile
        // shell with no colorant/TRC tags. Registered via the header-only
        // registry door (bypassing open_profile's deep parse) to prove the
        // lazy step-5 gate catches it at transform build.
        let icc = cms
            .registry_mut()
            .register_working_icc(synthetic_icc(b"mntr", b"RGB "), ColorModel::Rgba)
            .unwrap();
        let mut k = key(
            &cms,
            BuiltinSpace::Srgb,
            BuiltinSpace::Srgb,
            ColorModel::Rgba,
            ColorModel::Rgba,
        );
        k.dst = icc;
        let err = cms.transform(k).err().unwrap();
        assert!(
            matches!(
                err,
                ColorError::TransformBuild(_) | ColorError::ProfileMalformed(_)
            ),
            "expected a clean step-2/step-5 refusal, got {err:?}"
        );
        // Non-f32 layout.
        let mut k2 = key(
            &cms,
            BuiltinSpace::Srgb,
            BuiltinSpace::DisplayP3,
            ColorModel::Rgba,
            ColorModel::Rgba,
        );
        k2.fmt_in = PixelFormat::new(ColorModel::Rgba, Depth::U8);
        assert!(matches!(
            cms.transform(k2),
            Err(ColorError::UnsupportedFormat(_))
        ));
        // f16 layout: the paused-depth error, specifically.
        let mut k3 = key(
            &cms,
            BuiltinSpace::Srgb,
            BuiltinSpace::DisplayP3,
            ColorModel::Rgba,
            ColorModel::Rgba,
        );
        k3.fmt_out = PixelFormat::new(ColorModel::Rgba, Depth::F16);
        assert_eq!(
            cms.transform(k3).err(),
            Some(ColorError::DepthPaused(Depth::F16))
        );
        // Unknown uid.
        let mut k4 = key(
            &cms,
            BuiltinSpace::Srgb,
            BuiltinSpace::Srgb,
            ColorModel::Rgb,
            ColorModel::Rgb,
        );
        k4.src = ProfileUid::of_icc_bytes(b"ghost");
        k4.dst = k4.src;
        assert!(matches!(
            cms.transform(k4),
            Err(ColorError::UnknownProfile(_))
        ));
        // CMYK endpoints: profile-driven only (§5.5) — a CMYK layout
        // against an RGB profile is the §5.2 validation-4 mismatch (the
        // CP-1.0 machinery replaced the old wholesale CMYK refusal).
        let mut k5 = key(
            &cms,
            BuiltinSpace::Srgb,
            BuiltinSpace::DisplayP3,
            ColorModel::Cmyk,
            ColorModel::Rgb,
        );
        k5.fmt_in = PixelFormat::new(ColorModel::Cmyk, Depth::F32);
        assert_eq!(
            cms.transform(k5).err(),
            Some(ColorError::ProfileSpaceMismatch)
        );
    }

    /// Buffer-length mismatch is a structured error, not a panic.
    #[test]
    fn length_mismatch_is_refused() {
        let mut cms = Cms::new();
        let t = cms
            .transform(key(
                &cms,
                BuiltinSpace::Srgb,
                BuiltinSpace::DisplayP3,
                ColorModel::Rgb,
                ColorModel::Rgb,
            ))
            .unwrap();
        let src = [0.5f32; 7]; // not a multiple of 3
        let mut dst = [0f32; 7];
        assert!(t.apply_f32(&src, &mut dst).is_err());
        let src2 = [0.5f32; 6];
        let mut dst2 = [0f32; 3]; // pixel-count mismatch
        assert!(t.apply_f32(&src2, &mut dst2).is_err());
    }

    // ------------------------------------------------------------------
    // ICC-backed paths (lcms2 per deps/littlecms2.yaml; flag elections
    // NOCACHE + COPY_ALPHA (+ BPC per key) — icc.rs module docs).
    // ------------------------------------------------------------------

    /// Real ICC profile bytes for tests, synthesized through lcms2 itself
    /// (no third-party profile bytes enter the repo).
    fn srgb_icc_bytes() -> Vec<u8> {
        lcms2::Profile::new_srgb().icc().unwrap()
    }

    fn gray22_icc_bytes() -> Vec<u8> {
        let d50 = crate::chromatic::WhitePoint::D50.xyz();
        let sum = d50[0] + d50[1] + d50[2];
        let white = lcms2::CIExyY {
            x: d50[0] / sum,
            y: d50[1] / sum,
            Y: 1.0,
        };
        let curve = lcms2::ToneCurve::new(2.2);
        lcms2::Profile::new_gray(&white, &curve)
            .unwrap()
            .icc()
            .unwrap()
    }

    /// §5.2 ingress: `open_profile` runs the full ladder — truncated
    /// garbage dies at the header gate; a header-only shell either dies at
    /// the CMM deep parse or (if the CMM tolerates the shell) at lazy
    /// transform build; real synthesized bytes pass.
    #[test]
    fn open_profile_validation_ladder() {
        let mut cms = Cms::new();
        // Truncated garbage: header gate.
        assert!(matches!(
            cms.open_profile(vec![0u8; 50], ColorModel::Rgba),
            Err(ColorError::ProfileMalformed(_))
        ));
        // Header-only shell: deep parse or lazy build refusal.
        match cms.open_profile(synthetic_icc(b"mntr", b"RGB "), ColorModel::Rgba) {
            Err(ColorError::ProfileMalformed(_)) => {} // CMM rejected the shell at parse
            Ok(uid) => {
                let mut k = key(
                    &cms,
                    BuiltinSpace::Srgb,
                    BuiltinSpace::Srgb,
                    ColorModel::Rgba,
                    ColorModel::Rgba,
                );
                k.dst = uid;
                let err = cms.transform(k).err().unwrap();
                assert!(
                    matches!(
                        err,
                        ColorError::TransformBuild(_) | ColorError::ProfileMalformed(_)
                    ),
                    "{err:?}"
                );
            }
            Err(e) => panic!("unexpected ingress error: {e:?}"),
        }
        // Real bytes pass the whole ladder.
        let uid = cms
            .open_profile(srgb_icc_bytes(), ColorModel::Rgba)
            .unwrap();
        assert!(cms.registry().contains(uid));
        // Space mismatch is still enforced on real bytes.
        assert_eq!(
            cms.open_profile(srgb_icc_bytes(), ColorModel::GrayA).err(),
            Some(ColorError::ProfileSpaceMismatch)
        );
    }

    /// ICC-path evidence 1: builtin sRGB → user-registered sRGB profile
    /// bytes (lcms2's own sRGB) is appearance-identity — output within
    /// float-path tolerance of the input; alpha bit-untouched
    /// (COPY_ALPHA).
    #[test]
    fn icc_path_srgb_to_srgb_bytes_is_appearance_identity() {
        let mut cms = Cms::new();
        let user = cms
            .open_profile(srgb_icc_bytes(), ColorModel::Rgba)
            .unwrap();
        let mut k = key(
            &cms,
            BuiltinSpace::Srgb,
            BuiltinSpace::Srgb,
            ColorModel::Rgba,
            ColorModel::Rgba,
        );
        k.dst = user;
        let t = cms.transform(k).unwrap();
        assert!(
            !t.is_identity(),
            "distinct profile identities must not null-collapse"
        );
        let mut state = SEED;
        let mut src = Vec::new();
        for _ in 0..256 {
            src.push(next01(&mut state));
            src.push(next01(&mut state));
            src.push(next01(&mut state));
            src.push(next01(&mut state));
        }
        let mut dst = vec![0f32; src.len()];
        t.apply_f32(&src, &mut dst).unwrap();
        for px in 0..256 {
            for c in 0..3 {
                let a = src[px * 4 + c];
                let b = dst[px * 4 + c];
                assert!((a - b).abs() < 2e-3, "px {px} ch {c}: {a} vs {b}");
            }
            assert_eq!(
                src[px * 4 + 3].to_bits(),
                dst[px * 4 + 3].to_bits(),
                "COPY_ALPHA must pass alpha through bit-exact"
            );
        }
    }

    /// ICC-path evidence 2: pure matrix path vs the lcms2 path on the SAME
    /// definitions (builtin sRGB → Display P3 constants, one side compiled
    /// pure, one side through a synthesized P3 ICC profile). Agreement
    /// bound: encoded ≤ 2e-3 and ΔE76 < 0.5 on seeded in-gamut pixels
    /// (sRGB ⊂ P3, so no gamut clipping) — cross-DEFINITION agreement, not
    /// the §5.4 same-pair fast-path gate (< 0.1), since the two sides
    /// derive their matrices independently (Appendix B vs lcms2's
    /// chromaticity pipeline).
    #[test]
    fn icc_path_agrees_with_the_matrix_path() {
        let p3_bytes = crate::icc::test_builtin_icc_bytes(BuiltinSpace::DisplayP3);
        let mut cms = Cms::new();
        let user_p3 = cms.open_profile(p3_bytes, ColorModel::Rgb).unwrap();
        let mut k_icc = key(
            &cms,
            BuiltinSpace::Srgb,
            BuiltinSpace::Srgb,
            ColorModel::Rgb,
            ColorModel::Rgb,
        );
        k_icc.dst = user_p3;
        let t_icc = cms.transform(k_icc).unwrap();
        let t_pure = cms
            .transform(key(
                &cms,
                BuiltinSpace::Srgb,
                BuiltinSpace::DisplayP3,
                ColorModel::Rgb,
                ColorModel::Rgb,
            ))
            .unwrap();
        let mut state = SEED;
        let src: Vec<f32> = (0..900).map(|_| next01(&mut state)).collect();
        let mut icc_out = vec![0f32; src.len()];
        let mut pure_out = vec![0f32; src.len()];
        t_icc.apply_f32(&src, &mut icc_out).unwrap();
        t_pure.apply_f32(&src, &mut pure_out).unwrap();
        let mut max_enc = 0f32;
        let mut max_de = 0f64;
        let m = BuiltinSpace::DisplayP3.rgb_to_xyz_d50().unwrap();
        for px in 0..300 {
            let mut lin_icc = [0f64; 3];
            let mut lin_pure = [0f64; 3];
            for c in 0..3 {
                let a = icc_out[px * 3 + c];
                let b = pure_out[px * 3 + c];
                let d = (a - b).abs();
                if d > max_enc {
                    max_enc = d;
                }
                lin_icc[c] = TransferFn::SrgbPiecewise.decode(f64::from(a));
                lin_pure[c] = TransferFn::SrgbPiecewise.decode(f64::from(b));
            }
            // ΔE76 via the P3 → XYZ(D50) → Lab pipeline (both outputs are
            // encoded P3).
            let lab_a =
                crate::lab::xyz_to_lab(m.mul_vec(lin_icc), crate::chromatic::WhitePoint::D50);
            let lab_b =
                crate::lab::xyz_to_lab(m.mul_vec(lin_pure), crate::chromatic::WhitePoint::D50);
            let de = crate::lab::delta_e76(lab_a, lab_b);
            if de > max_de {
                max_de = de;
            }
        }
        assert!(max_enc < 2e-3, "max encoded diff {max_enc}");
        assert!(max_de < 0.5, "max deltaE76 {max_de}");
    }

    /// ICC-path evidence 3 (gray): user gray-gamma-2.2 bytes → builtin
    /// sGray through lcms2 vs the pure GrayGamma22 → SGray path.
    #[test]
    fn icc_gray_path_agrees_with_the_pure_gray_path() {
        let mut cms = Cms::new();
        let user_gray = cms
            .open_profile(gray22_icc_bytes(), ColorModel::Gray)
            .unwrap();
        let mut k_icc = key(
            &cms,
            BuiltinSpace::SGray,
            BuiltinSpace::SGray,
            ColorModel::Gray,
            ColorModel::Gray,
        );
        k_icc.src = user_gray;
        let t_icc = cms.transform(k_icc).unwrap();
        let t_pure = cms
            .transform(key(
                &cms,
                BuiltinSpace::GrayGamma22,
                BuiltinSpace::SGray,
                ColorModel::Gray,
                ColorModel::Gray,
            ))
            .unwrap();
        let mut state = SEED;
        let src: Vec<f32> = (0..512).map(|_| next01(&mut state)).collect();
        let mut icc_out = vec![0f32; src.len()];
        let mut pure_out = vec![0f32; src.len()];
        t_icc.apply_f32(&src, &mut icc_out).unwrap();
        t_pure.apply_f32(&src, &mut pure_out).unwrap();
        for (i, (a, b)) in icc_out.iter().zip(pure_out.iter()).enumerate() {
            assert!(
                (a - b).abs() < 2e-3,
                "sample {i}: {a} vs {b} (in {})",
                src[i]
            );
        }
    }

    /// Determinism across the lcms2 path (D-48): two independently built
    /// Cms instances, same profile bytes, same input → bit-identical
    /// output; and applying twice through one transform is bit-stable.
    #[test]
    fn icc_transform_is_deterministic_cross_instance() {
        let bytes = srgb_icc_bytes();
        let mut state = SEED;
        let src: Vec<f32> = (0..1200).map(|_| next01(&mut state)).collect();
        let run = || {
            let mut cms = Cms::new();
            let user = cms.open_profile(bytes.clone(), ColorModel::Rgba).unwrap();
            let mut k = key(
                &cms,
                BuiltinSpace::DisplayP3,
                BuiltinSpace::DisplayP3,
                ColorModel::Rgba,
                ColorModel::Rgba,
            );
            k.dst = user;
            let t = cms.transform(k).unwrap();
            let mut dst = vec![0f32; src.len()];
            t.apply_f32(&src, &mut dst).unwrap();
            let mut dst2 = vec![0f32; src.len()];
            t.apply_f32(&src, &mut dst2).unwrap();
            for (a, b) in dst.iter().zip(dst2.iter()) {
                assert_eq!(a.to_bits(), b.to_bits(), "same transform, same bytes");
            }
            dst
        };
        let a = run();
        let b = run();
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.to_bits(), y.to_bits(), "cross-instance determinism");
        }
    }

    /// NOCACHE concurrency: one Arc'd ICC transform applied from two
    /// threads produces bytes identical to each other and to a
    /// single-thread run (the §5.2 rationale for the flag).
    #[test]
    fn icc_concurrent_apply_is_stable() {
        let mut cms = Cms::new();
        let user = cms
            .open_profile(srgb_icc_bytes(), ColorModel::Rgba)
            .unwrap();
        let mut k = key(
            &cms,
            BuiltinSpace::DisplayP3,
            BuiltinSpace::DisplayP3,
            ColorModel::Rgba,
            ColorModel::Rgba,
        );
        k.dst = user;
        let t = cms.transform(k).unwrap();
        let mut state = SEED;
        let src: Vec<f32> = (0..2000).map(|_| next01(&mut state)).collect();
        let mut serial = vec![0f32; src.len()];
        t.apply_f32(&src, &mut serial).unwrap();
        let run = {
            let t = Arc::clone(&t);
            let src = src.clone();
            move || {
                let mut dst = vec![0f32; src.len()];
                t.apply_f32(&src, &mut dst).unwrap();
                dst
            }
        };
        let h1 = std::thread::spawn(run.clone());
        let h2 = std::thread::spawn(run);
        for out in [h1.join().unwrap(), h2.join().unwrap()] {
            for (a, b) in serial.iter().zip(out.iter()) {
                assert_eq!(a.to_bits(), b.to_bits());
            }
        }
    }

    // ------------------------------------------------------------------
    // Lab paths (CP-1.0 mode-convert machinery): the builtin CIE Lab
    // connection-space profile + the §5.2 normalized marshalling, end to
    // end through the real CMM.
    // ------------------------------------------------------------------

    /// RGB → Lab through the CMM matches the pure §5.5 math (decode TRC →
    /// XYZ(D50) → Lab → normalize) within cross-definition tolerance, on
    /// white, mid-gray, and the primaries.
    #[test]
    fn lab_path_agrees_with_the_pure_math() {
        let mut cms = Cms::new();
        let mut k = key(
            &cms,
            BuiltinSpace::Srgb,
            BuiltinSpace::Srgb,
            ColorModel::Rgb,
            ColorModel::Lab,
        );
        k.dst = cms.builtin_lab();
        let t = cms.transform(k).unwrap();
        let m = BuiltinSpace::Srgb.rgb_to_xyz_d50().unwrap();
        for px in [
            [1.0f32, 1.0, 1.0],
            [0.5, 0.5, 0.5],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.25, 0.5, 0.75],
        ] {
            let mut out = [0f32; 3];
            t.apply_f32(&px, &mut out).unwrap();
            // Pure oracle in real Lab values.
            let lin = [
                TransferFn::SrgbPiecewise.decode(f64::from(px[0])),
                TransferFn::SrgbPiecewise.decode(f64::from(px[1])),
                TransferFn::SrgbPiecewise.decode(f64::from(px[2])),
            ];
            let want = crate::lab::xyz_to_lab(m.mul_vec(lin), crate::chromatic::WhitePoint::D50);
            let got = crate::lab::lab_from_normalized([
                f64::from(out[0]),
                f64::from(out[1]),
                f64::from(out[2]),
            ]);
            let de = crate::lab::delta_e76(want, got);
            assert!(de < 0.5, "{px:?}: CMM {got:?} vs pure {want:?} (dE {de})");
        }
    }

    /// RgbA ↔ LabA round trip (the §6.2 RGB→Lab→RGB chain): seeded
    /// pixels return within float tolerance; alpha bit-exact both ways
    /// (COPY_ALPHA across the Lab marshalling).
    #[test]
    fn lab_round_trip_preserves_pixels_and_alpha() {
        let mut cms = Cms::new();
        let lab = cms.builtin_lab();
        let mut fwd_k = key(
            &cms,
            BuiltinSpace::Srgb,
            BuiltinSpace::Srgb,
            ColorModel::Rgba,
            ColorModel::LabA,
        );
        fwd_k.dst = lab;
        let fwd = cms.transform(fwd_k).unwrap();
        let mut back_k = key(
            &cms,
            BuiltinSpace::Srgb,
            BuiltinSpace::Srgb,
            ColorModel::LabA,
            ColorModel::Rgba,
        );
        back_k.src = lab;
        let back = cms.transform(back_k).unwrap();
        let mut state = SEED;
        let mut src = Vec::new();
        for _ in 0..256 {
            src.push(next01(&mut state));
            src.push(next01(&mut state));
            src.push(next01(&mut state));
            src.push(next01(&mut state)); // alpha
        }
        let mut mid = vec![0f32; src.len()];
        let mut rt = vec![0f32; src.len()];
        fwd.apply_f32(&src, &mut mid).unwrap();
        back.apply_f32(&mid, &mut rt).unwrap();
        for px in 0..256 {
            for c in 0..3 {
                let a = src[px * 4 + c];
                let b = rt[px * 4 + c];
                assert!((a - b).abs() < 3e-3, "px {px} ch {c}: {a} vs {b}");
            }
            assert_eq!(
                src[px * 4 + 3].to_bits(),
                mid[px * 4 + 3].to_bits(),
                "alpha through the Lab marshalling"
            );
            assert_eq!(src[px * 4 + 3].to_bits(), rt[px * 4 + 3].to_bits());
        }
        // Known value: encoded white → normalized Lab (1, 128/255,
        // 128/255) within tolerance (L* = 100, a* = b* = 0).
        let mut w = [0f32; 4];
        fwd.apply_f32(&[1.0, 1.0, 1.0, 1.0], &mut w).unwrap();
        assert!((w[0] - 1.0).abs() < 2e-3, "{w:?}");
        assert!((w[1] - 128.0 / 255.0).abs() < 2e-3, "{w:?}");
        assert!((w[2] - 128.0 / 255.0).abs() < 2e-3, "{w:?}");
    }

    /// Gray → Lab lands on the neutral axis (a* = b* = 0 within
    /// tolerance) — the §6.2 Gray→Lab row.
    #[test]
    fn gray_to_lab_is_neutral() {
        let mut cms = Cms::new();
        let mut k = key(
            &cms,
            BuiltinSpace::SGray,
            BuiltinSpace::SGray,
            ColorModel::Gray,
            ColorModel::Lab,
        );
        k.dst = cms.builtin_lab();
        let t = cms.transform(k).unwrap();
        for v in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
            let mut out = [0f32; 3];
            t.apply_f32(&[v], &mut out).unwrap();
            assert!((out[1] - 128.0 / 255.0).abs() < 2e-3, "{v}: {out:?}");
            assert!((out[2] - 128.0 / 255.0).abs() < 2e-3, "{v}: {out:?}");
        }
    }

    /// Lab-path determinism (the golden-pin property, D-48): two
    /// independently built engines produce bit-identical LabA bytes.
    #[test]
    fn lab_conversion_is_deterministic_cross_instance() {
        let mut state = SEED;
        let src: Vec<f32> = (0..1024).map(|_| next01(&mut state)).collect();
        let run = || {
            let mut cms = Cms::new();
            let mut k = key(
                &cms,
                BuiltinSpace::DisplayP3,
                BuiltinSpace::DisplayP3,
                ColorModel::Rgba,
                ColorModel::LabA,
            );
            k.dst = cms.builtin_lab();
            let t = cms.transform(k).unwrap();
            let mut dst = vec![0f32; src.len()];
            t.apply_f32(&src, &mut dst).unwrap();
            dst
        };
        let a = run();
        let b = run();
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.to_bits(), y.to_bits());
        }
    }

    /// The §5.4 cache serves ICC-backed keys like pure ones: same key →
    /// same Arc; invalidating the user profile drops its entries.
    #[test]
    fn icc_transforms_participate_in_the_cache() {
        let mut cms = Cms::new();
        let user = cms.open_profile(srgb_icc_bytes(), ColorModel::Rgb).unwrap();
        let mut k = key(
            &cms,
            BuiltinSpace::DisplayP3,
            BuiltinSpace::DisplayP3,
            ColorModel::Rgb,
            ColorModel::Rgb,
        );
        k.dst = user;
        let t1 = cms.transform(k).unwrap();
        let t2 = cms.transform(k).unwrap();
        assert!(Arc::ptr_eq(&t1, &t2));
        assert_eq!(cms.cached_transforms(), 1);
        cms.invalidate_profile(user);
        assert_eq!(cms.cached_transforms(), 0);
    }

    /// Transforms are shareable across threads (compile-time property +
    /// runtime smoke): apply the same Arc<Transform> from two threads and
    /// get identical bytes.
    #[test]
    fn transform_is_send_sync_and_stable_across_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Transform>();
        let mut cms = Cms::new();
        let t = cms
            .transform(key(
                &cms,
                BuiltinSpace::Srgb,
                BuiltinSpace::DisplayP3,
                ColorModel::Rgb,
                ColorModel::Rgb,
            ))
            .unwrap();
        let mut state = SEED;
        let src: Vec<f32> = (0..900).map(|_| next01(&mut state)).collect();
        let run = {
            let t = Arc::clone(&t);
            let src = src.clone();
            move || {
                let mut dst = vec![0f32; src.len()];
                t.apply_f32(&src, &mut dst).unwrap();
                dst
            }
        };
        let h1 = std::thread::spawn(run.clone());
        let h2 = std::thread::spawn(run);
        let a = h1.join().unwrap();
        let b = h2.join().unwrap();
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.to_bits(), y.to_bits());
        }
    }
}
