// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §29.1 "Export pipeline stages" [NORMATIVE] — the one
//! sequence every export path runs (stage 1, flatten, is upstream:
//! module doc of [`crate::export`]):
//!
//! 2. resize per `ResizeSpec` — Lanczos-3 down, bicubic up (§18.2),
//!    premultiplied; every scale variant re-runs from the flattened
//!    source;
//! 3. color convert per `ProfilePolicy` (§5.2 intent + BPC; the
//!    marshalling mirrors core's `doc.convert_profile` — straight-alpha
//!    f32 through the Cms, a == 0 stays exactly transparent);
//! 4. depth demotion — the codecs' §6.3 demotion; the §22.4 blue-noise
//!    table rides the recorded BN64 relocation ask (module doc);
//! 5. encode via the §8.1 registry with the preset's §8.2 options;
//! 6. metadata policy (§29.4, [`crate::export::policy`]);
//! 7. atomic write: `<name>.<ext>.part` + fsync + rename (§7.6
//!    semantics), §26.2 collision policy.
//!
//! File-size estimation (§29.1): a 1/16-area proxy (¼ linear scale)
//! encoded with the IDENTICAL option set, ×16, displayed with `~` —
//! the estimate never drops the tilde (the caller renders it; this
//! module returns the raw product).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use appname_color::{BuiltinSpace, Cms, ColorModel, Depth, Intent, PixelFormat, TransformKey};
use appname_core::IRect;
use appname_tools::resample::{resize_plane, EdgePolicy, Kernel, Plane};

use crate::error::{CodecError, IoDirection, IoFailure};
use crate::export::icc_synth;
use crate::export::policy;
use crate::export::preset::{ExportPreset, ProfilePolicy};
use crate::export::ExportSource;
use crate::opts::{EncodeOpts, IccPolicy};
use crate::registry::{CodecRegistry, CompatReport, ExportImage};

/// One encoded variant, in memory (stages 2–6).
#[derive(Debug)]
pub struct EncodedVariant {
    pub bytes: Vec<u8>,
    /// Post-processing pixel dimensions (§26.2 `{w}`/`{h}`).
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    /// The §29.4 / conversion ledger for this variant.
    pub report: CompatReport,
}

/// One written file (stage 7 outcome).
pub struct WrittenFile {
    pub path: PathBuf,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

/// Stages 2–6 for one `(rect, scale)` variant of a source. `rect` is a
/// canvas-space sub-rectangle (slices) or `None` for the full canvas.
pub fn encode_variant(
    source: &ExportSource<'_>,
    rect: Option<IRect>,
    preset: &ExportPreset,
    scale: f32,
    cms: &mut Cms,
) -> Result<EncodedVariant, CodecError> {
    source.check()?;
    if !(0.01..=16.0).contains(&scale) {
        return Err(CodecError::OptionOutOfRange {
            field: "export.scale",
            detail: format!("{scale} not in 0.01..=16"),
        });
    }
    let canvas = IRect::new(0, 0, source.width as i32, source.height as i32);
    let rect = match rect {
        None => canvas,
        Some(r) => {
            let clipped = r.intersect(&canvas);
            if clipped.is_empty() {
                return Err(CodecError::OptionOutOfRange {
                    field: "export.rect",
                    detail: "slice bounds do not intersect the canvas (§40.8 live-state rule)"
                        .into(),
                });
            }
            clipped
        }
    };

    // ---- crop (stage-1 tail: the slice rect view of the flatten) ----
    let (sw, sh) = (rect.width() as u32, rect.height() as u32);
    let mut plane = Plane::new(IRect::new(0, 0, sw as i32, sh as i32));
    for y in 0..sh as i32 {
        for x in 0..sw as i32 {
            let sx = (rect.min.x + x) as usize;
            let sy = (rect.min.y + y) as usize;
            let i = (sy * source.width as usize + sx) * 4;
            plane.set(
                x,
                y,
                [
                    source.plane[i],
                    source.plane[i + 1],
                    source.plane[i + 2],
                    source.plane[i + 3],
                ],
            );
        }
    }

    // ---- stage 2: resize (ResizeSpec then the scale variant), ONE
    // resampling pass from the flattened source ----
    let (base_w, base_h) = match preset.resize {
        Some(spec) => spec.target(sw, sh),
        None => (sw, sh),
    };
    let (dst_w, dst_h) = (
        ((base_w as f64 * scale as f64).round() as u32).max(1),
        ((base_h as f64 * scale as f64).round() as u32).max(1),
    );
    let plane = if (dst_w, dst_h) != (sw, sh) {
        // §29.1 stage 2 letter: Lanczos-3 down, bicubic (Catmull-Rom)
        // up; kernel chosen by the dominant axis ratio (axes only
        // diverge by rounding).
        let kernel = if dst_w < sw {
            Kernel::Lanczos3
        } else {
            Kernel::CatmullRom
        };
        resize_plane(
            &plane,
            IRect::new(0, 0, dst_w as i32, dst_h as i32),
            kernel,
            EdgePolicy::Clamp,
        )
    } else {
        plane
    };

    // ---- stage 3: color convert per ProfilePolicy ----
    let mut report = CompatReport::default();
    let mut px: Vec<f32> = Vec::with_capacity(plane.px.len() * 4);
    for p in &plane.px {
        px.extend_from_slice(p);
    }
    let (px, embed_profile): (Vec<f32>, Option<Vec<u8>>) = match &preset.profile_policy {
        ProfilePolicy::Embed => (px, source.icc.map(<[u8]>::to_vec)),
        ProfilePolicy::ConvertToSrgbStrip => {
            let converted = convert_pixels(
                px,
                source.icc,
                ConvertDest::Builtin(BuiltinSpace::Srgb),
                cms,
                &mut report,
            )?;
            (converted, None) // §29.4: only this policy omits the profile
        }
        ProfilePolicy::ConvertTo(space) => {
            let converted = convert_pixels(
                px,
                source.icc,
                ConvertDest::Builtin(*space),
                cms,
                &mut report,
            )?;
            // §29.1 [NORMATIVE]: an exported file embeds the correct
            // profile — the synthesized builtin definition (§5.2 legal
            // posture).
            (converted, Some(icc_synth::builtin_icc_bytes(*space)))
        }
    };

    // ---- stage 6 (policy) runs BEFORE encode because the codecs embed
    // at encode time; §29.4 ICC row: the profile is stage 3's outcome,
    // never the metadata policy's ----
    let filtered = policy::apply_policy(source.meta, preset.metadata_policy);
    report.items.extend(filtered.report.items);
    let mut meta = filtered.meta;
    meta.icc = embed_profile.clone();

    // ---- stages 4+5: depth demotion + encode (§6.3 demotion lives in
    // the codec encode paths; BN64 dithering rides the recorded ask) ----
    let registry = CodecRegistry::builtin();
    let codec = registry.by_id(preset.codec_id()).ok_or_else(|| {
        CodecError::internal(format!("codec `{}` unregistered", preset.codec_id()))
    })?;
    let img = ExportImage {
        width: dst_w,
        height: dst_h,
        plane: &px,
        layers: None,
        profile: embed_profile.as_deref(),
        meta: &meta,
    };
    let mut opts = EncodeOpts::new(preset.options.clone());
    // the policy matrix already filtered `meta`; the codec embeds what
    // survives (opts.rs: codecs consume the Preserve/not split)
    opts.metadata = crate::opts::MetadataPolicy::Preserve;
    opts.icc = if embed_profile.is_some() {
        IccPolicy::Embed
    } else {
        IccPolicy::Omit
    };
    opts.ppi = Some(source.ppi);
    let bytes = codec.encode(&img, &opts)?;

    Ok(EncodedVariant {
        bytes,
        width: dst_w,
        height: dst_h,
        scale,
        report,
    })
}

/// Stage-3 destination: a builtin working space (presets) or raw ICC
/// bytes (the §29.3 print chain — PDF takes the printer/paper profile
/// as bytes).
pub enum ConvertDest<'a> {
    Builtin(BuiltinSpace),
    IccBytes(&'a [u8]),
}

/// The PDF emitter's conversion door: same stage-3 machinery, returning
/// the converted plane instead of an encode.
pub fn convert_plane_for_pdf(
    source: &ExportSource<'_>,
    cms: &mut Cms,
    dest: ConvertDest<'_>,
) -> Result<Vec<f32>, CodecError> {
    source.check()?;
    let mut report = CompatReport::default();
    convert_pixels(source.plane.to_vec(), source.icc, dest, cms, &mut report)
}

/// Stage-3 pixel conversion: working (source profile or the interim
/// sRGB assumption) → the destination, mirroring core's
/// `doc.convert_profile` marshalling (straight-alpha f32; a == 0 stays
/// exactly transparent; §6.4).
fn convert_pixels(
    px: Vec<f32>,
    src_icc: Option<&[u8]>,
    dst: ConvertDest<'_>,
    cms: &mut Cms,
    report: &mut CompatReport,
) -> Result<Vec<f32>, CodecError> {
    let src_uid = match src_icc {
        None => cms.builtin(BuiltinSpace::Srgb), // documented interim assumption
        Some(bytes) => cms
            .open_profile(bytes.to_vec(), ColorModel::Rgba)
            .map_err(|e| CodecError::unsupported(format!("export profile convert: {e}")))?,
    };
    let (dst_uid, dst_desc) = match dst {
        ConvertDest::Builtin(space) => (cms.builtin(space), format!("{space:?}")),
        ConvertDest::IccBytes(bytes) => (
            cms.open_profile(bytes.to_vec(), ColorModel::Rgba)
                .map_err(|e| CodecError::unsupported(format!("export profile convert: {e}")))?,
            "destination ICC profile".to_string(),
        ),
    };
    let fmt = PixelFormat {
        model: ColorModel::Rgba,
        depth: Depth::F32,
    };
    let key = TransformKey {
        src: src_uid,
        dst: dst_uid,
        proof: None,
        // §29.1 stage 3: the document rendering intent + BPC (§5.2);
        // the document intent rides the source at the CLI-wiring row —
        // relative colorimetric + BPC is the §5.2 default pair.
        intent: Intent::RelativeColorimetric,
        bpc: true,
        fmt_in: fmt,
        fmt_out: fmt,
    };
    let transform = cms
        .transform(key)
        .map_err(|e| CodecError::unsupported(format!("export profile convert: {e}")))?;
    if transform.is_identity() {
        return Ok(px);
    }
    report.push(
        "export.color",
        crate::registry::CompatAction::Info,
        format!("stage-3 conversion to {dst_desc} (relative colorimetric + BPC)"),
    );
    // unpremul → transform → re-premul (the doc.convert_profile shape)
    let n = px.len() / 4;
    let mut straight = vec![0.0f32; px.len()];
    let mut alphas = vec![0.0f32; n];
    for i in 0..n {
        let a = px[i * 4 + 3];
        alphas[i] = a;
        let inv = if a > 0.0 { 1.0 / a } else { 0.0 };
        straight[i * 4] = px[i * 4] * inv;
        straight[i * 4 + 1] = px[i * 4 + 1] * inv;
        straight[i * 4 + 2] = px[i * 4 + 2] * inv;
        straight[i * 4 + 3] = a;
    }
    let mut out = vec![0.0f32; px.len()];
    transform
        .apply_f32(&straight, &mut out)
        .map_err(|e| CodecError::internal(format!("export transform: {e}")))?;
    for i in 0..n {
        let a = alphas[i];
        if a > 0.0 {
            out[i * 4] = (out[i * 4] * a).clamp(0.0, a);
            out[i * 4 + 1] = (out[i * 4 + 1] * a).clamp(0.0, a);
            out[i * 4 + 2] = (out[i * 4 + 2] * a).clamp(0.0, a);
            out[i * 4 + 3] = a;
        } else {
            out[i * 4] = 0.0;
            out[i * 4 + 1] = 0.0;
            out[i * 4 + 2] = 0.0;
            out[i * 4 + 3] = 0.0;
        }
    }
    Ok(out)
}

/// Stage 7 [NORMATIVE]: encode to `<final-name>.part` in the
/// destination directory, fsync, rename over the target (§7.6
/// semantics). Cancellation/crash leaves the target absent or the
/// pre-existing file intact — a stale `.part` MAY remain (W21 step 6).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CodecError> {
    let io_err = |e: std::io::Error| {
        CodecError::Io(IoFailure {
            source: e,
            during: IoDirection::Encode,
        })
    };
    let part = {
        let mut name = path
            .file_name()
            .ok_or_else(|| CodecError::internal("write_atomic: path has no file name"))?
            .to_os_string();
        name.push(".part");
        path.with_file_name(name)
    };
    let mut f = std::fs::File::create(&part).map_err(io_err)?;
    f.write_all(bytes).map_err(io_err)?;
    f.sync_all().map_err(io_err)?;
    drop(f);
    std::fs::rename(&part, path).map_err(io_err)?;
    Ok(())
}

/// Export-As entry: run every scale variant of `preset` for the full
/// canvas and write `{name}[@Nx].{ext}` into `dest` (stage 7 + §26.2
/// collision policy). Returns the written files in variant order.
pub fn export_file(
    source: &ExportSource<'_>,
    preset: &ExportPreset,
    dest: &Path,
    overwrite: bool,
    cms: &mut Cms,
) -> Result<Vec<WrittenFile>, CodecError> {
    let mut out = Vec::new();
    let mut claimed = BTreeMap::new();
    for &scale in &preset.scale_variants {
        let v = encode_variant(source, None, preset, scale, cms)?;
        let base = format!(
            "{}.{}",
            crate::export::naming::guard_value(source.doc_name),
            preset.extension()
        );
        let named = crate::export::naming::apply_scale_suffix(&base, scale);
        // DEF-1 final containment check — every path producer runs it
        // on the RESOLVED name before any write (ruled fix; the name
        // here is internally composed, so this is defense-in-depth).
        crate::export::naming::check_contained(&named)?;
        let path = crate::export::naming::resolve_collision(dest, &named, overwrite, &mut claimed);
        write_atomic(&path, &v.bytes)?;
        out.push(WrittenFile {
            path,
            bytes: v.bytes.len() as u64,
            width: v.width,
            height: v.height,
            scale,
        });
    }
    Ok(out)
}

/// §29.1 file-size estimation: encode a ¼-linear-scale proxy (1/16
/// area) with the IDENTICAL option set and multiply by 16. The caller
/// displays it with a leading `~` — the tilde is never dropped.
pub fn estimate_file_size(
    source: &ExportSource<'_>,
    preset: &ExportPreset,
    cms: &mut Cms,
) -> Result<u64, CodecError> {
    let mut proxy = preset.clone();
    proxy.scale_variants = vec![1.0];
    let v = encode_variant(source, None, &proxy, 0.25, cms)?;
    Ok(v.bytes.len() as u64 * 16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::Metadata;

    fn checker_source(w: u32, h: u32) -> (Vec<f32>, Metadata) {
        let mut plane = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let on = (x / 4 + y / 4) % 2 == 0;
                let v = if on { 1.0 } else { 0.25 };
                plane.extend_from_slice(&[v, v * 0.5, 0.1, 1.0]);
            }
        }
        (plane, Metadata::default())
    }

    fn src<'a>(plane: &'a [f32], meta: &'a Metadata, w: u32, h: u32) -> ExportSource<'a> {
        ExportSource {
            width: w,
            height: h,
            plane,
            icc: None,
            meta,
            doc_name: "doc",
            ppi: 72.0,
        }
    }

    #[test]
    fn variant_dimensions_follow_resize_and_scale() {
        let (plane, meta) = checker_source(64, 32);
        let s = src(&plane, &meta, 64, 32);
        let mut cms = Cms::new();
        let mut p = ExportPreset::default_for("png").unwrap();
        p.resize = Some(crate::export::preset::ResizeSpec::Width(32));
        let v = encode_variant(&s, None, &p, 2.0, &mut cms).unwrap();
        assert_eq!((v.width, v.height), (64, 32), "resize 32 wide × @2x");
        assert_eq!(v.scale, 2.0);
    }

    #[test]
    fn determinism_same_inputs_same_bytes() {
        let (plane, meta) = checker_source(48, 48);
        let s = src(&plane, &meta, 48, 48);
        let mut cms = Cms::new();
        let p = ExportPreset::default_for("png").unwrap();
        let a = encode_variant(&s, None, &p, 1.0, &mut cms).unwrap();
        let b = encode_variant(&s, None, &p, 1.0, &mut cms).unwrap();
        assert_eq!(a.bytes, b.bytes);
    }

    #[test]
    fn atomic_write_leaves_no_part_file() {
        let dir = std::env::temp_dir().join(format!("appname-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("out.png");
        write_atomic(&target, b"bytes").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"bytes");
        assert!(
            !dir.join("out.png.part").exists(),
            "stage 7: the staging file was renamed away"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn slice_rect_outside_canvas_rejects() {
        let (plane, meta) = checker_source(16, 16);
        let s = src(&plane, &meta, 16, 16);
        let mut cms = Cms::new();
        let p = ExportPreset::default_for("png").unwrap();
        let e = encode_variant(&s, Some(IRect::new(100, 100, 110, 110)), &p, 1.0, &mut cms)
            .unwrap_err();
        assert!(matches!(e, CodecError::OptionOutOfRange { .. }), "{e}");
    }

    #[test]
    fn estimate_is_sixteen_times_a_proxy() {
        let (plane, meta) = checker_source(64, 64);
        let s = src(&plane, &meta, 64, 64);
        let mut cms = Cms::new();
        let p = ExportPreset::default_for("png").unwrap();
        let est = estimate_file_size(&s, &p, &mut cms).unwrap();
        assert!(est > 0 && est.is_multiple_of(16));
    }
}
