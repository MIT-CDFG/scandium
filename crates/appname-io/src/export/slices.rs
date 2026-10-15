// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §29.2 "Web slices" — named rectangles producing multiple
//! assets from one document; [NORMATIVE] naming (§26.2 grammar with
//! `{slice}`, default template `{name}-{slice}.{ext}`, `@2x`/`@3x`
//! stems, traversal guard, collision policy) and the [NORMATIVE]
//! export manifest (`appname.slicemanifest.v1`, EVERY emitted file
//! listed). Acceptance: W15.
//!
//! Slices are DOCUMENT DATA (§29.2: serialized in `document.json`,
//! mutated only through `slice.add`/`slice.edit`/`slice.delete` — the
//! doc-model storage + kind impls are core's; the recorded core-door
//! ask in the lane log carries the exact signatures). This module is
//! the IO-SIDE machinery: given the flattened source and the slice
//! list (however core hands it over), it runs the §29.1 pipeline per
//! slice × format × scale and writes the manifest next to the outputs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use appname_color::Cms;
use appname_core::IRect;

use crate::error::CodecError;
use crate::export::naming::{self, TemplateVars};
use crate::export::pipeline::{self, write_atomic};
use crate::export::preset::ExportPreset;
use crate::export::ExportSource;

/// §29.2 `Slice` (the doc-model row, io-side view). `preset: None` =
/// the job's default preset at export time (`slice.add` contract
/// `defaults` note).
#[derive(Debug, Clone)]
pub struct SliceDef {
    pub name: String,
    pub bounds: IRect,
    pub preset: Option<ExportPreset>,
}

/// Caller-injected manifest identity (determinism: tests pin bytes,
/// production stamps a fresh UUIDv4 + RFC-3339 clock — W15's masked
/// compare covers both fields).
#[derive(Debug, Clone)]
pub struct ManifestStamp {
    /// UUID string for the manifest `id` field.
    pub id: String,
    /// RFC-3339 timestamp for the `generated` field.
    pub generated: String,
    /// Document file name for the `document` field.
    pub document: String,
}

/// One emitted file (manifest entry shape).
#[derive(Debug, Clone)]
pub struct SliceOutput {
    pub slice: String,
    pub path: PathBuf,
    pub file: String,
    pub scale: f32,
    pub width: u32,
    pub height: u32,
    pub format: &'static str,
    pub bytes: u64,
}

/// One REJECTED output (the DEF-1 containment check): typed per-file
/// failure, listed in the manifest with `"status": "failed"`, never a
/// write.
#[derive(Debug, Clone)]
pub struct SliceFailure {
    pub slice: String,
    /// The offending RESOLVED name (as composed; nothing was written).
    pub file: String,
    pub scale: f32,
    pub format: &'static str,
    /// Stable §8.1 MessageId of the rejection.
    pub message_id: &'static str,
    pub detail: String,
}

/// The slice-export outcome: written files + per-file containment
/// rejections (DEF-1 ruled shape — a rejection fails the FILE, not the
/// job; the manifest lists both).
#[derive(Debug, Clone)]
pub struct SliceExportReport {
    pub written: Vec<SliceOutput>,
    pub failed: Vec<SliceFailure>,
}

/// One slice-export job's destination knobs (grouped so the door keeps
/// one obvious argument list).
#[derive(Debug, Clone)]
pub struct SliceJob<'a> {
    /// §26.2 template; `None` = `{name}-{slice}.{ext}` (§29.2).
    pub template: Option<&'a str>,
    /// Destination directory (outputs + `export-manifest.json`).
    pub dest: &'a Path,
    /// §26.2 collision policy override.
    pub overwrite: bool,
    /// Manifest identity (caller-injected; module doc).
    pub stamp: ManifestStamp,
}

/// Export every slice through `presets` (one entry per requested output
/// format — W15 exports each slice as PNG AND WebP) at each preset's
/// scale variants; write outputs + `export-manifest.json` into
/// `job.dest`.
///
/// Unknown template variables abort BEFORE any file is written (§26.2 /
/// W15 step 6: validation error, no files). A resolved name failing the
/// DEF-1 containment check ([`naming::check_contained`]) is a typed
/// PER-FILE failure — listed in the manifest as failed, never written,
/// the rest of the job unaffected.
pub fn export_slices(
    source: &ExportSource<'_>,
    slices: &[SliceDef],
    presets: &[ExportPreset],
    job: &SliceJob<'_>,
    cms: &mut Cms,
) -> Result<SliceExportReport, CodecError> {
    source.check()?;
    if slices.is_empty() {
        return Err(CodecError::OptionOutOfRange {
            field: "export.slices",
            detail: "no slices to export".into(),
        });
    }
    let (dest, overwrite, stamp) = (job.dest, job.overwrite, &job.stamp);
    let template = job.template.unwrap_or(naming::DEFAULT_SLICE_TEMPLATE);

    // §26.2: template validation is a VALIDATION error — probe-run the
    // substitution for every (slice, preset) pair BEFORE writing
    // anything (W15 step 6: "validation error, no files written").
    for slice in slices {
        for preset in presets {
            let effective = slice.preset.as_ref().unwrap_or(preset);
            let vars = TemplateVars {
                name: source.doc_name.to_string(),
                ext: effective.extension().to_string(),
                slice: Some(slice.name.clone()),
                seq: None,
                w: Some(0),
                h: Some(0),
                date_ymd: None,
            };
            naming::substitute(template, &vars)?;
        }
    }

    let mut outputs: Vec<SliceOutput> = Vec::new();
    let mut failed: Vec<SliceFailure> = Vec::new();
    let mut claimed = BTreeMap::new();
    for slice in slices {
        for preset in presets {
            let effective = slice.preset.as_ref().unwrap_or(preset);
            for &scale in &effective.scale_variants {
                let v =
                    pipeline::encode_variant(source, Some(slice.bounds), effective, scale, cms)?;
                let vars = TemplateVars {
                    name: source.doc_name.to_string(),
                    ext: effective.extension().to_string(),
                    slice: Some(slice.name.clone()),
                    seq: None,
                    w: Some(v.width),
                    h: Some(v.height),
                    date_ymd: None,
                };
                let base = naming::substitute(template, &vars)?;
                let named = naming::apply_scale_suffix(&base, scale);
                // DEF-1 FINAL CONTAINMENT CHECK (ruled fix): the
                // COMPOSED name, before any filesystem effect. A
                // rejection fails this file, not the job.
                if let Err(e) = naming::check_contained(&named) {
                    failed.push(SliceFailure {
                        slice: slice.name.clone(),
                        file: named,
                        scale,
                        format: effective.codec_id(),
                        message_id: e.message_id(),
                        detail: e.to_string(),
                    });
                    continue;
                }
                let path = naming::resolve_collision(dest, &named, overwrite, &mut claimed);
                if let Some(parent) = path.parent() {
                    // contained by the check above; legit subdir
                    // templates (`web/{name}.{ext}`) materialize here
                    std::fs::create_dir_all(parent).map_err(|e| {
                        CodecError::Io(crate::error::IoFailure {
                            source: e,
                            during: crate::error::IoDirection::Encode,
                        })
                    })?;
                }
                write_atomic(&path, &v.bytes)?;
                let file = path
                    .strip_prefix(dest)
                    .expect("containment checked: resolved under dest")
                    .to_string_lossy()
                    .replace('\\', "/");
                outputs.push(SliceOutput {
                    slice: slice.name.clone(),
                    path,
                    file,
                    scale,
                    width: v.width,
                    height: v.height,
                    format: effective.codec_id(),
                    bytes: v.bytes.len() as u64,
                });
            }
        }
    }

    // §29.2 [NORMATIVE]: every slice export writes the manifest next to
    // the outputs, listing EVERY emitted file — and (DEF-1 ruling)
    // every containment rejection as a failed entry.
    let manifest = manifest_json(stamp, &outputs, &failed);
    write_atomic(
        &dest.join("export-manifest.json"),
        manifest.to_string().as_bytes(),
    )?;
    Ok(SliceExportReport {
        written: outputs,
        failed,
    })
}

/// The `appname.slicemanifest.v1` document (§29.2 example shape;
/// success entries keep the example's exact field set — `status`
/// appears ONLY on failed entries, so masked compares of the normative
/// example stay stable).
fn manifest_json(
    stamp: &ManifestStamp,
    outputs: &[SliceOutput],
    failed: &[SliceFailure],
) -> serde_json::Value {
    let mut entries: Vec<serde_json::Value> = outputs
        .iter()
        .map(|o| {
            serde_json::json!({
                "slice": o.slice,
                "file": o.file,
                "scale": o.scale,
                "width": o.width,
                "height": o.height,
                "format": o.format,
                "bytes": o.bytes,
            })
        })
        .collect();
    entries.extend(failed.iter().map(|f| {
        serde_json::json!({
            "slice": f.slice,
            "file": f.file,
            "scale": f.scale,
            "format": f.format,
            "status": "failed",
            "error": { "messageId": f.message_id, "detail": f.detail },
        })
    }));
    serde_json::json!({
        "schema": "appname.slicemanifest.v1",
        "id": stamp.id,
        "document": stamp.document,
        "generated": stamp.generated,
        "entries": entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::Metadata;

    fn flat_plane(w: u32, h: u32) -> Vec<f32> {
        let mut plane = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                plane.extend_from_slice(&[x as f32 / w as f32, y as f32 / h as f32, 0.5, 1.0]);
            }
        }
        plane
    }

    fn stamp() -> ManifestStamp {
        ManifestStamp {
            id: "00000000-0000-4000-8000-00000000aa01".into(),
            generated: "2026-08-17T00:00:00Z".into(),
            document: "hero.appname".into(),
        }
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("appname-slices-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn matrix_emits_every_file_and_the_manifest() {
        let plane = flat_plane(64, 32);
        let meta = Metadata::default();
        let source = ExportSource {
            width: 64,
            height: 32,
            plane: &plane,
            icc: None,
            meta: &meta,
            doc_name: "hero",
            ppi: 72.0,
        };
        let slices = vec![
            SliceDef {
                name: "navbar".into(),
                bounds: IRect::new(0, 0, 64, 8),
                preset: None,
            },
            SliceDef {
                name: "body".into(),
                bounds: IRect::new(0, 8, 64, 32),
                preset: None,
            },
        ];
        let mut png = ExportPreset::default_for("png").unwrap();
        png.scale_variants = vec![1.0, 2.0];
        let mut webp = ExportPreset::default_for("webp").unwrap();
        webp.scale_variants = vec![1.0, 2.0];
        let dest = tmpdir("matrix");
        let mut cms = Cms::new();
        let out = export_slices(
            &source,
            &slices,
            &[png, webp],
            &SliceJob {
                template: None,
                dest: &dest,
                overwrite: true,
                stamp: stamp(),
            },
            &mut cms,
        )
        .unwrap();
        // 2 slices × 2 formats × 2 scales = 8 files
        assert_eq!(out.written.len(), 8);
        assert!(out.failed.is_empty());
        assert!(dest.join("hero-navbar.png").exists());
        assert!(dest.join("hero-navbar@2x.png").exists());
        assert!(dest.join("hero-body.webp").exists());
        assert!(dest.join("hero-body@2x.webp").exists());
        // manifest lists every emitted file
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dest.join("export-manifest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["schema"], "appname.slicemanifest.v1");
        let entries = manifest["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 8, "§29.2: EVERY emitted file listed");
        for o in &out.written {
            assert!(
                entries.iter().any(|e| e["file"] == o.file.as_str()
                    && e["slice"] == o.slice.as_str()
                    && e["bytes"] == o.bytes),
                "{} in manifest",
                o.file
            );
        }
        // @2x doubled dimensions
        let nav2 = out
            .written
            .iter()
            .find(|o| o.file == "hero-navbar@2x.png")
            .unwrap();
        assert_eq!((nav2.width, nav2.height), (128, 16));
        std::fs::remove_dir_all(&dest).ok();
    }

    #[test]
    fn traversal_slice_name_stays_inside_dest_and_collisions_suffix() {
        let plane = flat_plane(16, 16);
        let meta = Metadata::default();
        let source = ExportSource {
            width: 16,
            height: 16,
            plane: &plane,
            icc: None,
            meta: &meta,
            doc_name: "hero",
            ppi: 72.0,
        };
        let slices = vec![SliceDef {
            name: "../evil".into(),
            bounds: IRect::new(0, 0, 16, 16),
            preset: None,
        }];
        let png = ExportPreset::default_for("png").unwrap();
        let dest = tmpdir("guard");
        let mut cms = Cms::new();
        let out = export_slices(
            &source,
            &slices,
            std::slice::from_ref(&png),
            &SliceJob {
                template: None,
                dest: &dest,
                overwrite: false,
                stamp: stamp(),
            },
            &mut cms,
        )
        .unwrap();
        assert_eq!(
            out.written[0].file, "hero-__evil.png",
            "traversal replaced with _"
        );
        assert!(
            out.written[0].path.starts_with(&dest),
            "stays inside the destination"
        );
        // re-export without overwrite → ` (2)` collision suffix (W15
        // step 5)
        let out2 = export_slices(
            &source,
            &slices,
            std::slice::from_ref(&png),
            &SliceJob {
                template: None,
                dest: &dest,
                overwrite: false,
                stamp: stamp(),
            },
            &mut cms,
        )
        .unwrap();
        assert_eq!(out2.written[0].file, "hero-__evil (2).png");
        std::fs::remove_dir_all(&dest).ok();
    }

    /// DEF-1 regression, proven escape (1) VERBATIM: a slice named `.`
    /// composes with `{slice}{slice}/…` into `../…` — per-value
    /// guarding passes each `.` untouched, only the composed result
    /// shows the traversal. The final containment check must reject it
    /// as a per-file failure: nothing written outside dest, the
    /// manifest lists it as failed.
    #[test]
    fn def1_composed_dot_slice_cannot_escape() {
        let plane = flat_plane(8, 8);
        let meta = Metadata::default();
        let source = ExportSource {
            width: 8,
            height: 8,
            plane: &plane,
            icc: None,
            meta: &meta,
            doc_name: "hero",
            ppi: 72.0,
        };
        let slices = vec![SliceDef {
            name: ".".into(),
            bounds: IRect::new(0, 0, 8, 8),
            preset: None,
        }];
        let png = ExportPreset::default_for("png").unwrap();
        let dest = tmpdir("def1-dot").join("inner");
        std::fs::create_dir_all(&dest).unwrap();
        let escaped_target = dest.parent().unwrap().join("escaped.png");
        std::fs::remove_file(&escaped_target).ok();
        let mut cms = Cms::new();
        let out = export_slices(
            &source,
            &slices,
            std::slice::from_ref(&png),
            &SliceJob {
                template: Some("{slice}{slice}/escaped.{ext}"),
                dest: &dest,
                overwrite: true,
                stamp: stamp(),
            },
            &mut cms,
        )
        .unwrap();
        assert!(
            !escaped_target.exists(),
            "DEF-1: `../escaped.png` must never be written outside dest"
        );
        assert!(out.written.is_empty());
        assert_eq!(out.failed.len(), 1);
        assert_eq!(out.failed[0].file, "../escaped.png");
        // the manifest lists the rejection as failed
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dest.join("export-manifest.json")).unwrap())
                .unwrap();
        let entries = manifest["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["status"], "failed");
        assert_eq!(entries[0]["file"], "../escaped.png");
        std::fs::remove_dir_all(dest.parent().unwrap()).ok();
    }

    /// DEF-1 regression, proven escape (2) VERBATIM: an absolute
    /// template makes `dest.join` discard dest entirely. The final
    /// containment check rejects the absolute resolved name per file.
    #[test]
    fn def1_absolute_template_cannot_escape() {
        let plane = flat_plane(8, 8);
        let meta = Metadata::default();
        let source = ExportSource {
            width: 8,
            height: 8,
            plane: &plane,
            icc: None,
            meta: &meta,
            doc_name: "hero",
            ppi: 72.0,
        };
        let slices = vec![SliceDef {
            name: "a".into(),
            bounds: IRect::new(0, 0, 8, 8),
            preset: None,
        }];
        let png = ExportPreset::default_for("png").unwrap();
        let dest = tmpdir("def1-abs");
        let outside = std::path::PathBuf::from(format!(
            "/tmp/appname-def1-escape-{}-a.png",
            std::process::id()
        ));
        std::fs::remove_file(&outside).ok();
        let template = format!(
            "/tmp/appname-def1-escape-{}-{{slice}}.{{ext}}",
            std::process::id()
        );
        let mut cms = Cms::new();
        let out = export_slices(
            &source,
            &slices,
            std::slice::from_ref(&png),
            &SliceJob {
                template: Some(&template),
                dest: &dest,
                overwrite: true,
                stamp: stamp(),
            },
            &mut cms,
        )
        .unwrap();
        assert!(
            !outside.exists(),
            "DEF-1: an absolute template must never write outside dest"
        );
        assert!(out.written.is_empty());
        assert_eq!(out.failed.len(), 1);
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dest.join("export-manifest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["entries"][0]["status"], "failed");
        std::fs::remove_dir_all(&dest).ok();
    }

    /// DEF-1 positive case: literal RELATIVE subdir template text is
    /// legitimate — `web/{name}-{slice}.{ext}` writes INSIDE dest, the
    /// subdirectory materializes, and the manifest lists the
    /// dest-relative name.
    #[test]
    fn def1_legit_subdir_template_writes_inside_dest() {
        let plane = flat_plane(8, 8);
        let meta = Metadata::default();
        let source = ExportSource {
            width: 8,
            height: 8,
            plane: &plane,
            icc: None,
            meta: &meta,
            doc_name: "hero",
            ppi: 72.0,
        };
        let slices = vec![SliceDef {
            name: "a".into(),
            bounds: IRect::new(0, 0, 8, 8),
            preset: None,
        }];
        let png = ExportPreset::default_for("png").unwrap();
        let dest = tmpdir("def1-subdir");
        let mut cms = Cms::new();
        let out = export_slices(
            &source,
            &slices,
            std::slice::from_ref(&png),
            &SliceJob {
                template: Some("web/{name}-{slice}.{ext}"),
                dest: &dest,
                overwrite: true,
                stamp: stamp(),
            },
            &mut cms,
        )
        .unwrap();
        assert!(out.failed.is_empty());
        assert_eq!(out.written.len(), 1);
        assert_eq!(out.written[0].file, "web/hero-a.png");
        assert!(dest.join("web/hero-a.png").exists());
        assert!(out.written[0].path.starts_with(&dest));
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dest.join("export-manifest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["entries"][0]["file"], "web/hero-a.png");
        assert!(manifest["entries"][0].get("status").is_none());
        std::fs::remove_dir_all(&dest).ok();
    }

    #[test]
    fn unknown_template_variable_writes_nothing() {
        let plane = flat_plane(8, 8);
        let meta = Metadata::default();
        let source = ExportSource {
            width: 8,
            height: 8,
            plane: &plane,
            icc: None,
            meta: &meta,
            doc_name: "hero",
            ppi: 72.0,
        };
        let slices = vec![SliceDef {
            name: "a".into(),
            bounds: IRect::new(0, 0, 8, 8),
            preset: None,
        }];
        let png = ExportPreset::default_for("png").unwrap();
        let dest = tmpdir("badvar");
        let mut cms = Cms::new();
        let err = export_slices(
            &source,
            &slices,
            std::slice::from_ref(&png),
            &SliceJob {
                template: Some("{name}-{artboard}.{ext}"),
                dest: &dest,
                overwrite: true,
                stamp: stamp(),
            },
            &mut cms,
        )
        .unwrap_err();
        assert!(matches!(err, CodecError::OptionOutOfRange { .. }));
        let entries: Vec<_> = std::fs::read_dir(&dest).unwrap().collect();
        assert!(
            entries.is_empty(),
            "W15 step 6: validation error, NO files written"
        );
        std::fs::remove_dir_all(&dest).ok();
    }

    #[test]
    fn slice_pixels_equal_flatten_then_crop() {
        let plane = flat_plane(32, 32);
        let meta = Metadata::default();
        let source = ExportSource {
            width: 32,
            height: 32,
            plane: &plane,
            icc: None,
            meta: &meta,
            doc_name: "d",
            ppi: 72.0,
        };
        let mut cms = Cms::new();
        let png = ExportPreset::default_for("png").unwrap();
        // whole-canvas encode of the crop region == slice encode
        let rect = IRect::new(8, 8, 24, 24);
        let sliced = pipeline::encode_variant(&source, Some(rect), &png, 1.0, &mut cms).unwrap();
        // hand-built reference source for the same rect
        let mut sub = Vec::new();
        for y in 8..24u32 {
            for x in 8..24u32 {
                let i = ((y * 32 + x) * 4) as usize;
                sub.extend_from_slice(&plane[i..i + 4]);
            }
        }
        let ref_source = ExportSource {
            width: 16,
            height: 16,
            plane: &sub,
            icc: None,
            meta: &meta,
            doc_name: "d",
            ppi: 72.0,
        };
        let reference = pipeline::encode_variant(&ref_source, None, &png, 1.0, &mut cms).unwrap();
        assert_eq!(
            sliced.bytes, reference.bytes,
            "W15 step 4: flatten-then-crop equality"
        );
    }
}
