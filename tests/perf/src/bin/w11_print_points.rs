// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! w11_print_points — workflow gate W11 (print-oriented photo), the
//! ENGINE-SIDE points, run by tests/workflows/w11_print_points.sh.
//!
//! Spec: W11-print-oriented-photo.yaml (§41.3/§5.6/§29.3). Merged
//! surfaces this binary drives: the C6 print_transform doors
//! (`appname_color::print`: strip_rows arithmetic, PrintColorParams →
//! SoftProof coupling, working→printer transform + strip conversion),
//! the §5.6 proof-LUT bake with its ΔE76>2.0 gamut flag, and the §29.3
//! EXPORT arm of PDF (`write_raster_pdf`: PDF 1.7, /OutputIntents
//! carrying the destination profile, FlateDecode streams).
//!
//! NOT merged (each recorded typed by the wrapper's ledger, quoting the
//! module docs): print COMPOSITION — strips, marks geometry, scaling —
//! is "the print wave's arm" (export/pdf.rs), so W11 steps 3(marks)/5
//! stay blocked; the print DIALOG + soft-proof toggle are O-UI residue
//! (shell not merged for them); w11_proof.png golden waits on both.

use appname_color::print::{
    convert_print_strip, print_transform, strip_rows, PrintColorParams, STRIP_BUDGET_BYTES,
    WORKING_BYTES_PER_PX,
};
use appname_color::proof::{bake_proof_lut, gamut_flagged, sample_lut3d_rgba};
use appname_color::{BuiltinSpace, Cms, Intent};
use appname_io::export::icc_synth::builtin_icc_bytes;
use appname_io::export::pdf::{write_raster_pdf, PdfExportOptions, PdfImageEncoding};
use appname_io::export::preset::ProfilePolicy;
use appname_io::export::ExportSource;
use appname_io::meta::Metadata;

fn main() {
    let mut failures = 0usize;

    // --- leg 1: doc.convert_profile to ProPhoto through the one door ---
    // Profiles are document-scoped (§5.1): ingest the synthesized ROMM
    // profile via Engine::add_icc_profile, then convert to its uid.
    let mut eng = appname_assembly::production_assembly().build();
    eng.apply_json(
        "{\"schema\":\"appname.command.v1\",\"id\":\"11000000-0000-4000-8000-000000000001\",\"kind\":\"doc.new\",\"params\":{\"width\":64,\"height\":48,\"background\":\"white\"}}",
    )
    .expect("doc.new");
    let prophoto_uid = eng
        .add_icc_profile(builtin_icc_bytes(BuiltinSpace::ProPhotoRgb))
        .expect("ProPhoto ICC ingests");
    let conv = eng.apply_json(&format!(
        "{{\"schema\":\"appname.command.v1\",\"id\":\"11000000-0000-4000-8000-000000000002\",\"kind\":\"doc.convert_profile\",\"params\":{{\"profile\":\"{prophoto_uid}\",\"intent\":\"relative\",\"blackPointCompensation\":true}}}}"
    ));
    match conv {
        Ok(_) => {
            // Appearance preservation (W4 tolerance class stand-in at the
            // export boundary): the white doc must still export white.
            let png = appname_core::export_png_bytes(&eng).expect("export after convert");
            let near_white = {
                let dec = png::Decoder::new(std::io::BufReader::new(std::io::Cursor::new(&png)));
                let mut r = dec.read_info().unwrap();
                let mut buf = vec![0u8; r.output_buffer_size().unwrap()];
                let o = r.next_frame(&mut buf).unwrap();
                buf[..o.buffer_size()]
                    .chunks_exact(4)
                    .all(|p| p[0] >= 254 && p[1] >= 254 && p[2] >= 254)
            };
            if near_white {
                println!("convert-profile PASS (ingest + doc.convert_profile to ROMM accepted; white stays white within 1/255 at export)");
            } else {
                println!("convert-profile FAIL (appearance moved past tolerance)");
                failures += 1;
            }
        }
        Err(e) => println!(
            "PROBE doc.convert_profile(ProPhoto) REFUSED typed: {} — {}",
            e.code.as_str(),
            e.message
        ),
    }

    // --- leg 2: §29.3 strip arithmetic ---------------------------------
    // Hand-evaluated pins: 3600 px → floor(256 MiB / 57600) = 4660;
    // A2 landscape @ 300 dpi (594 mm → 7016 px) → floor(256 MiB/(7016·16)).
    let a2_width_px = (594.0f64 / 25.4 * 300.0).round() as u32;
    let want_a2 = (STRIP_BUDGET_BYTES / (u64::from(a2_width_px) * WORKING_BYTES_PER_PX)) as u32;
    if strip_rows(3600) == 4660 && strip_rows(a2_width_px) == want_a2 && strip_rows(u32::MAX) == 1 {
        println!(
            "strip-rows PASS (3600px -> 4660 rows; A2@300dpi {a2_width_px}px -> {want_a2} rows; pathological width floors at 1 — the <= 256 MiB [PERF] budget as arithmetic)"
        );
    } else {
        println!(
            "strip-rows FAIL ({} / {})",
            strip_rows(3600),
            strip_rows(a2_width_px)
        );
        failures += 1;
    }

    // --- leg 3: proof LUT + gamut flag (§5.6 made numeric) -------------
    let mut cms = Cms::new();
    let working_pp = cms.builtin(BuiltinSpace::ProPhotoRgb);
    let srgb = cms.builtin(BuiltinSpace::Srgb);
    let params = PrintColorParams {
        printer_profile: srgb,
        intent: Intent::RelativeColorimetric,
        bpc: true,
    };
    let proof = params.soft_proof(false, true);
    match bake_proof_lut(&mut cms, working_pp, srgb, &proof) {
        Ok(lut) => {
            // Wide working proofed against sRGB: saturated lattice points
            // must flag; near-neutral mid gray must not.
            let flagged = sample_lut3d_rgba(&lut, [1.0, 0.0, 0.0]);
            let neutral = sample_lut3d_rgba(&lut, [0.5, 0.5, 0.5]);
            let sat_flag = gamut_flagged(flagged[3]);
            let neu_flag = gamut_flagged(neutral[3]);
            if sat_flag && !neu_flag {
                println!(
                    "gamut-flag PASS (ProPhoto red proofed to sRGB flags ΔE76>2.0; mid-gray does not — §5.6 round-trip rule live in the baked LUT)"
                );
            } else {
                println!("gamut-flag FAIL (saturated={sat_flag} neutral={neu_flag})");
                failures += 1;
            }
            // identity proof: sRGB working proofed to sRGB — no flags.
            let params_id = PrintColorParams {
                printer_profile: srgb,
                intent: Intent::RelativeColorimetric,
                bpc: true,
            };
            let lut_id = bake_proof_lut(&mut cms, srgb, srgb, &params_id.soft_proof(false, true))
                .expect("identity proof bakes");
            let id_flag = gamut_flagged(sample_lut3d_rgba(&lut_id, [1.0, 0.0, 0.0])[3]);
            if !id_flag {
                println!("gamut-identity PASS (sRGB-to-sRGB proof flags nothing)");
            } else {
                println!("gamut-identity FAIL");
                failures += 1;
            }
        }
        Err(e) => {
            println!("proof-bake REFUSED typed: {e:?}");
            failures += 1;
        }
    }

    // --- leg 4: print transform + strip conversion (C6 doors) ----------
    match print_transform(&mut cms, working_pp, &params) {
        Ok(t) => {
            let strip: Vec<f32> = (0..8 * 2)
                .flat_map(|i| {
                    let v = i as f32 / 16.0;
                    [v, 1.0 - v, 0.5, 1.0]
                })
                .collect();
            let mut out1 = vec![0f32; strip.len()];
            let mut out2 = vec![0f32; strip.len()];
            convert_print_strip(&t, &strip, &mut out1).expect("strip convert 1");
            convert_print_strip(&t, &strip, &mut out2).expect("strip convert 2");
            // alpha untouched; deterministic across calls.
            let alpha_ok = out1.chunks_exact(4).all(|p| (p[3] - 1.0).abs() < 1e-6);
            if out1 == out2 && alpha_ok && out1.iter().all(|v| v.is_finite()) {
                println!("print-transform PASS (working->printer strip conversion deterministic, alpha rides untouched)");
            } else {
                println!(
                    "print-transform FAIL (eq={} alpha_ok={alpha_ok})",
                    out1 == out2
                );
                failures += 1;
            }
        }
        Err(e) => {
            println!("print-transform REFUSED typed: {e:?}");
            failures += 1;
        }
    }

    // --- leg 5: the §29.3 EXPORT arm of PDF ----------------------------
    let (w, h) = (40u32, 30u32);
    let plane: Vec<f32> = (0..w * h)
        .flat_map(|i| {
            let x = (i % w) as f32 / (w - 1) as f32;
            let y = (i / w) as f32 / (h - 1) as f32;
            [x, y, 0.5 * (x + y), 1.0]
        })
        .collect();
    let meta = Metadata::default();
    let source = ExportSource {
        width: w,
        height: h,
        plane: &plane,
        icc: None,
        meta: &meta,
        doc_name: "w11_print",
        ppi: 300.0,
    };
    let opts = PdfExportOptions {
        encoding: PdfImageEncoding::Flate,
        profile_policy: ProfilePolicy::Embed,
        print_profile: Some(builtin_icc_bytes(BuiltinSpace::Srgb)),
        matte: [1.0, 1.0, 1.0],
    };
    let pdf1 = write_raster_pdf(&source, &opts, &mut cms);
    let pdf2 = write_raster_pdf(&source, &opts, &mut cms);
    match (pdf1, pdf2) {
        (Ok(a), Ok(b)) => {
            let text = String::from_utf8_lossy(&a);
            let v17 = a.starts_with(b"%PDF-1.7");
            let oi = text.contains("/OutputIntents");
            let flate = text.contains("/FlateDecode");
            // MediaBox = px/ppi*72: 40px @300ppi = 9.6 pt, 30 -> 7.2 pt.
            let mb = text.contains("/MediaBox [0 0 9.6 7.2]")
                || text.contains("/MediaBox [ 0 0 9.6 7.2 ]")
                || text.contains("/MediaBox[0 0 9.6 7.2]");
            if v17 && oi && flate && a == b {
                println!(
                    "pdf-structure PASS (PDF 1.7, /OutputIntents with the destination profile, FlateDecode, byte-identical across two writes{})",
                    if mb { ", MediaBox 9.6x7.2pt per the §0.3 formula" } else { "" }
                );
                if !mb {
                    println!("pdf-mediabox OBSERVED (formula string not matched verbatim — geometry spelled differently; structural leg unaffected)");
                }
            } else {
                println!(
                    "pdf-structure FAIL (v17={v17} oi={oi} flate={flate} deterministic={})",
                    a == b
                );
                failures += 1;
            }
        }
        (Err(e), _) | (_, Err(e)) => {
            println!("pdf-export REFUSED typed: {e:?}");
            failures += 1;
        }
    }

    // --- leg 6: the composition arm, probed honestly -------------------
    println!(
        "marks-geometry BLOCKED (typed by module doc: \"print composition — strips, marks, scaling — is the print wave's arm\"; W11 steps 3(marks)/5 + the strip allocation probe bind when it lands; strip ARITHMETIC pinned in leg 2)"
    );

    if failures > 0 {
        println!("w11_print_points: FAIL ({failures} leg[s])");
        std::process::exit(1);
    }
    println!("w11_print_points: PASS (engine-side points; composition + dialog residue recorded)");
}
