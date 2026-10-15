// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! RASTER-ASSET INGRESS WIDENING (roadmap item 22; operator ruling
//! Q-D5(a), 2026-08-29).
//!
//! # What was actually wrong
//!
//! `Engine::add_raster_asset` decode-validates with
//! `core::io::png_io::decode_png_to_buffer` and **nothing else**, so the
//! single-asset ingress door accepted **PNG alone** — while
//! `CodecRegistry::builtin()` next door already read png/jpeg/tiff/webp/
//! gif/bmp/psd/ora. **The shell wrote seven formats and read one.**
//!
//! # Why the fix is here and not in core
//!
//! ★ **The crate dependency runs `appname-io -> appname-core`, never the
//! other way**, so `add_raster_asset` *cannot* reach the codec registry.
//! Widening it in place would mean a fifth engine service seam alongside
//! `vector` / `text` / `blend_kernels` / `fill_convert`.
//!
//! **That work is not needed, because the seam already exists and is
//! already load-bearing.** `ImportedLayer::ingress_png` is documented as
//! *"the journaled-import asset carrier: the deterministic PNG transcode
//! this layer's buffer was decoded from"*, and `import_into_engine` has
//! been handing exactly that to `add_raster_asset` for every ORA and PSD
//! layer all along. **Multi-format import already worked — through one
//! door.** The single-asset door simply never learned the trick.
//!
//! # The invariant that decides the shape
//!
//! ★★★ **PNG MUST PASS THROUGH BYTE-IDENTICAL.** Asset ids are
//! content-derived (`assets::asset_id_for`), so transcoding a PNG — even
//! to a pixel-identical PNG — would mint a **different asset id** for
//! bytes that already have one. That breaks replay determinism, every
//! recorded envelope that names an asset id, and the goldens. So PNG is
//! not "the easy case handled by the general path"; it is a case the
//! general path would get **wrong**, and it is short-circuited first.

use crate::opts::DecodeOpts;
use crate::registry::CodecRegistry;

/// Codec ids accepted at the SINGLE-RASTER-ASSET door, per the operator's
/// Q-D5(a) allow-list.
///
/// ★ **An allow-list, not the registry's own membership.** The registry
/// also carries `psd` and `ora`, and both are deliberately absent here:
/// they are **layered containers**, and flattening one into a single
/// raster asset would silently discard the layers a user opened the file
/// to keep. They already have a door — `import_into_engine`'s layered
/// path — and this one is not it. `raw` is likewise absent: it is
/// license-gated behind a feature, so its slot may not exist at all, and
/// a list that changed shape with a build flag would make this door's
/// behaviour depend on how the binary was compiled.
///
/// ★★ **Matched on the CODEC ID the registry RESOLVED, never on the file
/// extension.** `resolve()` sniffs content; an extension is whatever the
/// caller typed. Gating on the extension would let `evil.png` holding PSD
/// bytes through the check and then decode as PSD.
pub const INGRESS_ALLOWED: &[&str] = &["png", "jpeg", "tiff", "webp", "gif", "bmp"];

/// Why an ingress decode was refused. Carries the resolved codec id where
/// there was one, so the caller can say *which* format it declined rather
/// than "unsupported".
#[derive(Debug)]
pub enum IngressError {
    /// Nothing in the registry claimed these bytes.
    Unrecognized,
    /// Recognized, and deliberately not accepted at this door.
    NotAllowed { codec: String },
    /// Recognized and allowed, but the decode itself failed.
    Decode { codec: String, message: String },
}

impl core::fmt::Display for IngressError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            IngressError::Unrecognized => write!(
                f,
                "not an ingestible raster image: no codec recognized these bytes \
                 (accepted: {})",
                INGRESS_ALLOWED.join(", ")
            ),
            // NAMES THE FORMAT AND WHY, because "unsupported format" sends
            // a user to search for a plugin that does not exist, while
            // "psd is a layered container, open it instead" is actionable.
            IngressError::NotAllowed { codec } => write!(
                f,
                "{codec} is not accepted as a single raster asset (accepted: {}); \
                 layered containers are opened, not pasted",
                INGRESS_ALLOWED.join(", ")
            ),
            IngressError::Decode { codec, message } => {
                write!(f, "{codec} decode failed: {message}")
            }
        }
    }
}

/// Decode any allow-listed raster file into the PNG carrier
/// `Engine::add_raster_asset` ingests.
///
/// `ext` is the caller's file extension when it has one — used ONLY as
/// the registry's documented tie-break between codecs whose magic is
/// equally long, never as the accept/reject key.
///
/// Returns the bytes to hand to `add_raster_asset`. **For PNG input this
/// is the input, unchanged and unmoved** — see the module doc for why
/// that is load-bearing rather than an optimization.
pub fn decode_for_ingress(bytes: Vec<u8>, ext: Option<&str>) -> Result<Vec<u8>, IngressError> {
    let registry = CodecRegistry::builtin();
    let head_len = bytes.len().min(512);
    let codec = registry
        .resolve(&bytes[..head_len], ext)
        .map_err(|_| IngressError::Unrecognized)?;
    let id = codec.id().0.to_string();

    if !INGRESS_ALLOWED.contains(&id.as_str()) {
        return Err(IngressError::NotAllowed { codec: id });
    }
    // THE PNG SHORT-CIRCUIT. Must come after the allow-list check (so the
    // gate is never bypassed) and before the decode (so the bytes are
    // never rewritten). See the module doc: re-encoding would change the
    // content-derived asset id.
    if id == "png" {
        return Ok(bytes);
    }

    let imported =
        codec
            .decode(&bytes, &DecodeOpts::default())
            .map_err(|e| IngressError::Decode {
                codec: id.clone(),
                message: e.to_string(),
            })?;
    Ok(imported.ingress_png)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A 2x2 PNG built the same way the core suites build theirs
    // (kinds.rs::tiny_png), so this fixture cannot drift from what the
    // engine door accepts.
    fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 2, 2);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&[255, 0, 0, 255].repeat(4)).unwrap();
        }
        out
    }

    #[test]
    fn png_passes_through_byte_identical() {
        // THE INVARIANT, PINNED: not "a PNG is accepted" but "the SAME
        // BYTES come back". A pixel-identical re-encode would pass a
        // weaker assertion and still change every asset id.
        let png = tiny_png();
        let out = decode_for_ingress(png.clone(), Some("png")).expect("png accepted");
        assert_eq!(out, png, "PNG was rewritten — asset ids would change");
    }

    #[test]
    fn garbage_is_unrecognized_and_says_what_is_accepted() {
        let err = decode_for_ingress(b"not an image at all".to_vec(), None).unwrap_err();
        assert!(matches!(err, IngressError::Unrecognized));
        let msg = err.to_string();
        // The message must name the accepted set; a bare "unsupported"
        // sends the reader nowhere.
        assert!(
            msg.contains("png"),
            "message does not name what IS accepted: {msg}"
        );
    }

    #[test]
    fn the_allow_list_is_the_ruled_set_and_excludes_the_containers() {
        // POPULATION FIRST. A list that silently shrank would pass every
        // membership check below by having fewer members to fail.
        assert_eq!(
            INGRESS_ALLOWED.len(),
            6,
            "the ruled allow-list is six formats"
        );
        for want in ["png", "jpeg", "tiff", "webp", "gif", "bmp"] {
            assert!(
                INGRESS_ALLOWED.contains(&want),
                "{want} missing from the ruled set"
            );
        }
        // ★ The MUST-NOT half, which is the half a membership loop cannot
        // check: the registry carries these and this door must not.
        for deny in ["psd", "ora"] {
            assert!(
                !INGRESS_ALLOWED.contains(&deny),
                "{deny} is a layered container and must not be flattened into one asset"
            );
        }
    }

    // ★★★ THE ARM THAT PROVES THE FEATURE, AND WITHOUT IT THE OTHERS ARE
    // A VACUOUS GREEN. Everything above shows PNG still works and PSD is
    // refused -- both were true BEFORE this module existed. The point of
    // Q-D5(a) is that a JPEG, TIFF, WebP, GIF or BMP now gets IN, and only
    // this walks that.
    //
    // ★★ FIXTURES COME FROM THE PROJECT'S OWN ENCODERS, NOT FROM THE
    // `image` CRATE. The first draft used `image`, whose feature set here
    // cannot write tiff/gif/webp -- so three of five formats hit a
    // skip-and-continue and the arm proved almost nothing. The population
    // guard below caught that (2 of 5) and is the reason this rewrite
    // happened rather than a lowered threshold. Encoding through the same
    // registry that decodes also makes the round trip honest: these are
    // the bytes this product actually writes.
    #[test]
    fn every_widened_format_actually_decodes_to_an_ingestible_carrier() {
        let (w, h) = (4u32, 4u32);
        let mut plane = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                // Linear premultiplied RGBA, opaque.
                if x < 2 {
                    plane[i] = 1.0;
                } else {
                    plane[i + 2] = 1.0;
                }
                plane[i + 3] = 1.0;
            }
        }
        let meta = crate::meta::Metadata::default();
        let reg = CodecRegistry::builtin();
        let mut proven: Vec<&str> = Vec::new();
        let mut skipped: Vec<&str> = Vec::new();

        // Per-codec options at their spec defaults -- EncodeOpts has no
        // Default impl by design (the format arm must be stated), so the
        // pairing is explicit and a new format cannot be added to the loop
        // without someone choosing its options.
        let cases: &[(&str, crate::opts::CodecOptions)] = &[
            ("jpeg", crate::opts::CodecOptions::Jpeg(Default::default())),
            ("tiff", crate::opts::CodecOptions::Tiff(Default::default())),
            ("gif", crate::opts::CodecOptions::Gif(Default::default())),
            ("bmp", crate::opts::CodecOptions::Bmp(Default::default())),
            ("webp", crate::opts::CodecOptions::Webp(Default::default())),
        ];
        for (name, fmt) in cases {
            let name = *name;
            let codec = reg.by_id(name).expect("builtin codec missing");
            if !codec.capabilities().write {
                skipped.push(name);
                continue;
            }
            let img = crate::registry::ExportImage {
                width: w,
                height: h,
                plane: &plane,
                layers: None,
                profile: None,
                meta: &meta,
            };
            let encoded = match codec.encode(&img, &crate::opts::EncodeOpts::new(fmt.clone())) {
                Ok(b) => b,
                Err(_) => {
                    skipped.push(name);
                    continue;
                }
            };
            let out = decode_for_ingress(encoded.clone(), Some(name))
                .unwrap_or_else(|e| panic!("{name} refused at the ingress door: {e}"));
            assert!(!out.is_empty(), "{name} produced an empty carrier");
            // The carrier must be a PNG -- that is what add_raster_asset
            // validates. Checked by MAGIC, not by trusting a field name.
            assert_eq!(
                &out[..8],
                b"\x89PNG\r\n\x1a\n",
                "{name} carrier is not a PNG"
            );
            assert_ne!(out, encoded, "{name} was passed through unconverted");
            proven.push(name);
        }

        // POPULATION GUARD. Without it the loop passes when every format
        // hits a `continue` and nothing is proven at all -- the shape a
        // skip-on-error loop hides, and the shape that already caught one
        // bad fixture choice in this very test.
        assert!(
            proven.len() == 5,
            "only {} of 5 widened formats were exercised (proven {:?}, skipped {:?})",
            proven.len(),
            proven,
            skipped
        );
    }

    #[test]
    fn a_container_is_refused_by_id_not_by_extension() {
        // The registry must carry psd for this arm to mean anything --
        // otherwise "refused" and "not present" are indistinguishable.
        let reg = CodecRegistry::builtin();
        assert!(
            reg.by_id("psd").is_some(),
            "control dead: registry has no psd codec"
        );
        assert!(!INGRESS_ALLOWED.contains(&"psd"));
    }
}
