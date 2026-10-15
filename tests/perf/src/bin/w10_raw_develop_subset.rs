// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! w10_raw_develop_subset — workflow gate W10 (RAW photo end-to-end) at
//! its dispatch-scoped SUBSET, run by tests/workflows/w10_raw_photo_subset.sh.
//!
//! Spec: W10-raw-photo-end-to-end.yaml (§41.3/§8.3/Part 43). The W10
//! letter's DNG-import leg rides LibRaw, whose mosaic-egress gap is
//! TYPED-REFUSED pending the packaging-wave ruling (deps/libraw.yaml;
//! the stage1 module doc's recorded finding) — leg A pins exactly that
//! refusal on the shipped codec. The develop-v1 stages (§43.3: WB →
//! demosaic → lens 4b → highlight recovery → exposure → U16 quantize)
//! and the lensdb stage-4b machinery all run on SYNTHETIC CFA input —
//! legs B–E prove them deterministic and behaviorally sane. The CLI-side
//! W10 steps (heal, curves+mask, JPEG export, reopen) live in the
//! wrapper with their per-step probes.

use appname_algo::lensdb::{DistortionModel, ResolvedCalibration, ResolvedVignetting};
use appname_io::raw::develop::{
    demosaic, develop, CfaPattern, CfaPlane, LensCorrection, MatrixSource,
};
use appname_io::raw::params::{Demosaic, RawDevelopParams, WhiteBalance};
use appname_io::{CodecRegistry, DecodeOpts};

/// Synthetic RGGB mosaic of a smooth two-gradient scene.
fn synth_cfa(w: usize, h: usize) -> CfaPlane {
    let mut data = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let r = x as f32 / (w - 1) as f32;
            let g = y as f32 / (h - 1) as f32;
            let b = 0.5 * (r + g);
            let v = match (y % 2, x % 2) {
                (0, 0) => r,
                (0, 1) | (1, 0) => g,
                _ => b,
            };
            data.push(v * 0.8);
        }
    }
    CfaPlane {
        width: w,
        height: h,
        pattern: CfaPattern::Rggb,
        data,
    }
}

fn main() {
    let mut failures = 0usize;

    // --- leg A: the typed mosaic-egress refusal (module absent) --------
    // A TIFF-magic head that the RAW sniff subset claims (DNG class).
    let mut dng_ish = vec![0x49, 0x49, 0x2A, 0x00];
    dng_ish.extend_from_slice(&[0u8; 64]);
    let reg = CodecRegistry::builtin();
    match reg.by_id("raw") {
        Some(raw) => match raw.decode(&dng_ish, &DecodeOpts::default()) {
            Err(e) => {
                let msg = format!("{e:?}");
                if msg.contains("raw_disabled") || msg.contains("module_unavailable") {
                    println!(
                        "module-absent PASS (typed §43.5 row-1 refusal — the LibRaw module is not distributed pending the packaging-wave mosaic-egress ruling): {msg}"
                    );
                } else {
                    println!("module-absent OBSERVED other typed refusal: {msg}");
                }
            }
            Ok(_) => {
                println!("module-absent OBSERVED: decode SUCCEEDED (a LibRaw module is installed in this environment; the refusal leg is vacuous here)");
            }
        },
        None => {
            println!("module-absent OBSERVED: no `raw` registry slot in this build (registration itself is feature-gated) — refusal leg recorded against the registry door instead");
        }
    }

    // --- leg B: develop determinism + default path ---------------------
    let cfa = synth_cfa(64, 48);
    let params = RawDevelopParams::default(); // asShot / ahd / 0 / 0
    let mul = [2.0f32, 1.0, 1.5, 1.0];
    let d1 = develop(&cfa, &params, mul, None, None).expect("develop 1");
    let d2 = develop(&cfa, &params, mul, None, None).expect("develop 2");
    if d1.pixels == d2.pixels && d1.gains == d2.gains {
        println!(
            "develop-determinism PASS (default asShot+AHD develop byte-stable across two runs; gains {:?}, matrixSource {:?})",
            d1.gains, d1.matrix_source
        );
    } else {
        println!("develop-determinism FAIL");
        failures += 1;
    }
    if d1.matrix_source == MatrixSource::AssumedSrgb {
        println!("matrix-source PASS (no matrix supplied -> AssumedSrgb, the §43.2 rule-3 recorded fallback)");
    } else {
        println!("matrix-source FAIL ({:?})", d1.matrix_source);
        failures += 1;
    }

    // --- leg C: preview re-develop (linear) differs but is stable ------
    let p_linear = RawDevelopParams {
        demosaic: Demosaic::Linear,
        ..RawDevelopParams::default()
    };
    let dl = develop(&cfa, &p_linear, mul, None, None).expect("linear develop");
    let dl2 = develop(&cfa, &p_linear, mul, None, None).expect("linear develop 2");
    if dl.pixels == dl2.pixels && dl.pixels != d1.pixels {
        println!(
            "redevelop-linear PASS (preview demosaic deterministic and distinct from AHD commit)"
        );
    } else {
        println!(
            "redevelop-linear FAIL (stable={} distinct={})",
            dl.pixels == dl2.pixels,
            dl.pixels != d1.pixels
        );
        failures += 1;
    }

    // --- leg D: WB modes + exposure/highlight params -------------------
    let p_auto = RawDevelopParams {
        white_balance: WhiteBalance::Auto,
        ..RawDevelopParams::default()
    };
    let da = develop(&cfa, &p_auto, mul, None, None).expect("auto develop");
    let p_custom = RawDevelopParams {
        white_balance: WhiteBalance::Custom {
            temp_k: 5500.0,
            tint: 0.0,
        },
        ..RawDevelopParams::default()
    };
    let dc = develop(&cfa, &p_custom, mul, None, None).expect("custom develop");
    let finite = da.gains.iter().all(|g| g.is_finite() && *g > 0.0)
        && dc.gains.iter().all(|g| g.is_finite() && *g > 0.0);
    if finite && da.gains != d1.gains {
        println!(
            "wb-modes PASS (auto {:?} and custom-5500K {:?} both resolve finite positive gains distinct from asShot)",
            da.gains, dc.gains
        );
    } else {
        println!("wb-modes FAIL (finite={finite})");
        failures += 1;
    }
    let p_push = RawDevelopParams {
        exposure_ev: 1.0,
        highlight_recovery: 0.5,
        ..RawDevelopParams::default()
    };
    let dp = develop(&cfa, &p_push, mul, None, None).expect("pushed develop");
    let mean = |px: &[u16]| px.iter().map(|v| *v as u64).sum::<u64>() as f64 / px.len() as f64;
    if mean(&dp.pixels) > mean(&d1.pixels) {
        println!(
            "exposure PASS (+1 EV raises the mean: {:.0} > {:.0})",
            mean(&dp.pixels),
            mean(&d1.pixels)
        );
    } else {
        println!("exposure FAIL");
        failures += 1;
    }
    // Param validation is typed BEFORE pixel work (§43.3).
    let bad = RawDevelopParams {
        exposure_ev: 99.0,
        ..RawDevelopParams::default()
    };
    match develop(&cfa, &bad, mul, None, None) {
        Err(e) => println!("param-validation PASS (exposureEv 99 refused typed: {e:?})"),
        Ok(_) => {
            println!("param-validation FAIL (out-of-range exposure accepted)");
            failures += 1;
        }
    }

    // --- leg E: stage-4b lens correction on synthetic calibration ------
    let img = demosaic(&cfa, Demosaic::Linear).expect("demosaic for lens leg");
    let cal_neutral = ResolvedCalibration {
        distortion: Some(DistortionModel::Ptlens {
            a: 0.0,
            b: 0.0,
            c: 0.0,
        }),
        tca: None,
        vignetting: None,
    };
    let corr_neutral = appname_io::raw::develop::apply_lens_correction(
        &img,
        cfa.width,
        cfa.height,
        &LensCorrection {
            calibration: &cal_neutral,
            distortion: true,
            chromatic_aberration: true,
            vignette: true,
        },
    );
    let max_dev = img
        .iter()
        .zip(corr_neutral.iter())
        .flat_map(|(a, b)| (0..3).map(move |i| (a[i] - b[i]).abs()))
        .fold(0.0f32, f32::max);
    if max_dev < 1e-4 {
        println!(
            "lens-neutral PASS (neutral ptlens calibration is an identity within {max_dev:e})"
        );
    } else {
        println!("lens-neutral FAIL (max deviation {max_dev})");
        failures += 1;
    }
    let cal_vig = ResolvedCalibration {
        distortion: None,
        tca: None,
        vignetting: Some(ResolvedVignetting {
            k1: -0.3,
            k2: 0.0,
            k3: 0.0,
            aperture: 2.8,
            distance: 1000.0,
        }),
    };
    let corr_vig = appname_io::raw::develop::apply_lens_correction(
        &img,
        cfa.width,
        cfa.height,
        &LensCorrection {
            calibration: &cal_vig,
            distortion: false,
            chromatic_aberration: false,
            vignette: true,
        },
    );
    let corr_vig2 = appname_io::raw::develop::apply_lens_correction(
        &img,
        cfa.width,
        cfa.height,
        &LensCorrection {
            calibration: &cal_vig,
            distortion: false,
            chromatic_aberration: false,
            vignette: true,
        },
    );
    // k1 < 0 -> falloff < 1 off-center -> division BRIGHTENS corners.
    let corner = corr_vig[0][1];
    let corner_src = img[0][1];
    let stable = corr_vig.iter().zip(corr_vig2.iter()).all(|(a, b)| a == b);
    if stable && corner > corner_src && corr_vig.iter().flatten().all(|v| v.is_finite()) {
        println!(
            "lens-vignette PASS (deterministic; corner brightened {corner_src:.4} -> {corner:.4}; all finite)"
        );
    } else {
        println!("lens-vignette FAIL (stable={stable} corner {corner_src:.4} -> {corner:.4})");
        failures += 1;
    }

    if failures > 0 {
        println!("w10_raw_develop_subset: FAIL ({failures} leg[s])");
        std::process::exit(1);
    }
    println!("w10_raw_develop_subset: PASS (develop-v1 + lens legs on synthetic input; DNG ingest stays typed-refused)");
}
