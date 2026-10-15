// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Brush engine v1 determinism matrix (plan §2.2 acceptance: "fixed-seed
//! tests + a content-hash stability test over a stroke matrix (sizes ×
//! hardness × opacity × spacing × pressure curves)").
//!
//! Spec: ES §15.4 "Replayed strokes are bit-identical: same samples + same
//! seed ⇒ same dabs (tested, §34.4)"; §15.5 CPU-mirror discipline. The
//! frozen table pins the composited output of every matrix cell so any
//! accidental math drift (spline, march, curve eval, RNG schedule, tip
//! coverage, presentation) turns the suite red. The matrix avoids libm-
//! dependent paths (angle 0, no Direction curves) so its hashes are
//! platform-portable; see the `brush` module docs caveat.
//!
//! In-crate tests use computed oracles/hashes by design — image goldens are
//! O-QA's (plan preamble rule / §0.6 golden policy).

use appname_core::{BlendMode, IRect};
use appname_tools::brush::{
    commit_hash, plan_stroke, rasterize_plan, render_brush_stroke, BrushDynamics, BrushPreset,
    BrushStrokeSpec, DynamicCurve, InputSample, PaintOp, PresentOptions, ScatterParams,
    StrokeContext, StrokeParams, UniformBackdrop,
};

/// A fixed wavy diagonal stroke with a pressure ramp and varying velocity —
/// exercises the spline (non-collinear), the dynamics lerp, and multi-tile
/// output. Pure integer-derived f64s: identical bytes on every platform.
fn matrix_samples() -> Vec<InputSample> {
    (0..16)
        .map(|i| {
            let fi = i as f64;
            InputSample {
                x: 8.0 + fi * 20.0,
                y: 40.0 + ((i % 4) as f64 - 1.5) * 18.0,
                pressure: (0.15 + 0.85 * (i as f32 / 15.0)).min(1.0),
                tilt_x: 0.25,
                tilt_y: -0.125,
                rotation: 0.5,
                velocity: 300.0 + (i % 5) as f32 * 350.0,
                t: fi * 0.008,
            }
        })
        .collect()
}

fn preset_for(diameter: f32, hardness: f32, spacing: f32, dynamics: &str) -> BrushPreset {
    let dyn_table = match dynamics {
        "none" => BrushDynamics::default(),
        "size" => BrushDynamics {
            size: vec![DynamicCurve::pressure_linear()],
            ..BrushDynamics::default()
        },
        "size+flow" => BrushDynamics {
            size: vec![DynamicCurve::pressure_linear()],
            flow: vec![DynamicCurve::pressure_linear()],
            ..BrushDynamics::default()
        },
        other => panic!("unknown dynamics variant {other}"),
    };
    BrushPreset {
        diameter,
        spacing,
        hardness,
        roundness: 1.0,
        angle_rad: 0.0,
        scatter: ScatterParams::default(),
        dynamics: dyn_table,
    }
}

const SIZES: [f32; 3] = [4.0, 32.0, 150.0];
const HARDNESS: [f32; 3] = [0.0, 0.5, 1.0];
const OPACITY: [f32; 2] = [0.35, 1.0];
const SPACING: [f32; 3] = [0.05, 0.25, 1.0];
const DYNAMICS: [&str; 3] = ["none", "size", "size+flow"];
const SEED: u64 = 0x5EED_0001;
const CLIP: IRect = IRect::new(-64, -64, 512, 256);

/// One matrix cell → `(name, "dabcount:hash16")`. Plan + raster are shared
/// across the opacity axis (opacity is presentation-only, §15.5 step 4), so
/// the suite stays affordable inside the per-commit gate.
fn cell(
    diameter: f32,
    hardness: f32,
    opacity: f32,
    spacing: f32,
    dynamics: &str,
) -> (String, String) {
    let samples = matrix_samples();
    let preset = preset_for(diameter, hardness, spacing, dynamics);
    let spec = BrushStrokeSpec {
        samples: &samples,
        preset: &preset,
        size_px: None,
        hardness: None,
        opacity,
        flow: 0.9,
        color: [0.8, 0.35, 0.1],
        blend: BlendMode::Normal,
        seed: SEED,
    };
    let ctx = StrokeContext {
        clip: CLIP,
        selection: None,
        kernels: None,
        clamp_to_backdrop_alpha: false,
    };
    let outcome = render_brush_stroke(&spec, &UniformBackdrop([0.0; 4]), &ctx).unwrap();
    let hash = commit_hash(&outcome.commit).to_hex();
    (
        format!("d{diameter}_h{hardness}_o{opacity}_s{spacing}_{dynamics}"),
        format!("{}:{}", outcome.plan.dabs.len(), &hash[..16]),
    )
}

fn full_matrix() -> Vec<(String, String)> {
    let samples = matrix_samples();
    let mut rows = Vec::new();
    for d in SIZES {
        for h in HARDNESS {
            for s in SPACING {
                for dy in DYNAMICS {
                    // Plan + raster once per (d, h, s, dyn); present per
                    // opacity. Equivalence with the one-shot orchestrator is
                    // pinned by `phases_compose_to_the_orchestrator_result`
                    // and the spot cells in `matrix_spot_cells_replay`.
                    let preset = preset_for(d, h, s, dy);
                    let params = StrokeParams {
                        size_px: None,
                        hardness: None,
                        opacity: 1.0,
                        flow: 0.9,
                        seed: SEED,
                    };
                    let plan = plan_stroke(&samples, &preset, &params).unwrap();
                    let scratch = rasterize_plan(&plan, CLIP);
                    let op = PaintOp::Paint {
                        color: [0.8, 0.35, 0.1],
                        blend: BlendMode::Normal,
                        kernels: None,
                    };
                    for o in OPACITY {
                        let opts = PresentOptions {
                            opacity: o,
                            selection: None,
                            clamp_to_backdrop_alpha: false,
                        };
                        let commit = appname_tools::brush::present(
                            &scratch,
                            &UniformBackdrop([0.0; 4]),
                            &op,
                            &opts,
                        )
                        .unwrap();
                        let hash = commit_hash(&commit).to_hex();
                        rows.push((
                            format!("d{d}_h{h}_o{o}_s{s}_{dy}"),
                            format!("{}:{}", plan.dabs.len(), &hash[..16]),
                        ));
                    }
                }
            }
        }
    }
    rows
}

/// In-process replay: a spot set of full one-shot cells re-runs bit-for-bit
/// (§34.4). Cross-process/build replay is pinned by the frozen table.
#[test]
fn matrix_spot_cells_replay() {
    let spots = [
        (4.0, 0.0, 0.35, 0.05, "size+flow"),
        (32.0, 0.5, 1.0, 0.25, "size"),
        (150.0, 1.0, 0.35, 1.0, "none"),
    ];
    for (d, h, o, s, dy) in spots {
        assert_eq!(cell(d, h, o, s, dy), cell(d, h, o, s, dy));
    }
    // Sanity: distinct cells produce distinct output.
    let a = full_matrix();
    let mut hashes: Vec<&String> = a.iter().map(|(_, v)| v).collect();
    hashes.sort();
    hashes.dedup();
    assert!(
        hashes.len() > a.len() / 2,
        "matrix cells must not collapse to one output"
    );
}

/// Seed sensitivity: with jitter in play, a different seed changes output;
/// the same seed replays exactly (fixed-seed test of the plan row).
#[test]
fn seeded_jitter_replays_and_differs() {
    let samples = matrix_samples();
    let mut preset = preset_for(32.0, 0.5, 0.25, "size");
    preset.dynamics.size[0].jitter = 0.3;
    preset.scatter = ScatterParams {
        amount: 0.4,
        both_axes: true,
        count: 2,
        count_jitter: 0.25,
    };
    let mk = |seed: u64| {
        let spec = BrushStrokeSpec {
            samples: &samples,
            preset: &preset,
            size_px: None,
            hardness: None,
            opacity: 1.0,
            flow: 1.0,
            color: [0.2, 0.9, 0.4],
            blend: BlendMode::Normal,
            seed,
        };
        let ctx = StrokeContext {
            clip: CLIP,
            selection: None,
            kernels: None,
            clamp_to_backdrop_alpha: false,
        };
        let out = render_brush_stroke(&spec, &UniformBackdrop([0.0; 4]), &ctx).unwrap();
        commit_hash(&out.commit).to_hex()
    };
    assert_eq!(mk(42), mk(42));
    assert_ne!(mk(42), mk(43));
}

/// Phase-level replay: the plan (dab sequence) itself is bitwise stable and
/// the scratch → present split equals the orchestrator (seam equivalence).
#[test]
fn phases_compose_to_the_orchestrator_result() {
    let samples = matrix_samples();
    let preset = preset_for(32.0, 0.5, 0.25, "size+flow");
    let params = StrokeParams {
        size_px: None,
        hardness: None,
        opacity: 0.7,
        flow: 0.9,
        seed: SEED,
    };
    let plan_a = plan_stroke(&samples, &preset, &params).unwrap();
    let plan_b = plan_stroke(&samples, &preset, &params).unwrap();
    assert_eq!(
        plan_a.dabs, plan_b.dabs,
        "dab sequence replay is bitwise (§15.4/§34.4)"
    );

    let scratch = rasterize_plan(&plan_a, CLIP);
    let op = PaintOp::Paint {
        color: [0.8, 0.35, 0.1],
        blend: BlendMode::Normal,
        kernels: None,
    };
    let opts = PresentOptions {
        opacity: 0.7,
        selection: None,
        clamp_to_backdrop_alpha: false,
    };
    let manual =
        appname_tools::brush::present(&scratch, &UniformBackdrop([0.0; 4]), &op, &opts).unwrap();

    let spec = BrushStrokeSpec {
        samples: &samples,
        preset: &preset,
        size_px: None,
        hardness: None,
        opacity: 0.7,
        flow: 0.9,
        color: [0.8, 0.35, 0.1],
        blend: BlendMode::Normal,
        seed: SEED,
    };
    let ctx = StrokeContext {
        clip: CLIP,
        selection: None,
        kernels: None,
        clamp_to_backdrop_alpha: false,
    };
    let orchestrated = render_brush_stroke(&spec, &UniformBackdrop([0.0; 4]), &ctx).unwrap();
    assert_eq!(
        commit_hash(&manual).as_bytes(),
        commit_hash(&orchestrated.commit).as_bytes(),
        "public phases are the orchestrator, exactly"
    );
}

/// The frozen table: regenerate with
/// `cargo test -p appname-tools --test brush_matrix -- --ignored --nocapture`
/// and paste. Any diff = the engine's wire behavior changed = spec-owner
/// decision required (module pins).
#[test]
#[ignore = "generator for the frozen table below"]
fn print_matrix_table() {
    for (k, v) in full_matrix() {
        println!("    (\"{k}\", \"{v}\"),");
    }
}

#[test]
fn matrix_matches_the_frozen_table() {
    let expected: &[(&str, &str)] = FROZEN;
    let got = full_matrix();
    assert_eq!(got.len(), expected.len(), "matrix shape changed");
    let mut bad = Vec::new();
    for ((gk, gv), (ek, ev)) in got.iter().zip(expected.iter()) {
        if gk != ek || gv != ev {
            bad.push(format!("{gk}: got {gv}, frozen {ek}={ev}"));
        }
    }
    assert!(bad.is_empty(), "matrix drift:\n{}", bad.join("\n"));
}

#[rustfmt::skip]
const FROZEN: &[(&str, &str)] = &[
    ("d4_h0_o0.35_s0.05_none", "2002:0aaee8477f32892b"),
    ("d4_h0_o1_s0.05_none", "2002:f75a9cc150b2b8f1"),
    ("d4_h0_o0.35_s0.05_size", "2002:46547c3235755181"),
    ("d4_h0_o1_s0.05_size", "2002:24c2c9c462c30cb9"),
    ("d4_h0_o0.35_s0.05_size+flow", "2002:d0f620bfb5da7c8c"),
    ("d4_h0_o1_s0.05_size+flow", "2002:2cfd1ab46fb8715e"),
    ("d4_h0_o0.35_s0.25_none", "501:d65cb5d268543221"),
    ("d4_h0_o1_s0.25_none", "501:6df7af1da0d1f724"),
    ("d4_h0_o0.35_s0.25_size", "1039:738d12b0214944c8"),
    ("d4_h0_o1_s0.25_size", "1039:f0b8cdc0683ee91b"),
    ("d4_h0_o0.35_s0.25_size+flow", "1039:042979e824d9dfe5"),
    ("d4_h0_o1_s0.25_size+flow", "1039:8a14693e6aa17251"),
    ("d4_h0_o0.35_s1_none", "126:db72de20e2a43bf5"),
    ("d4_h0_o1_s1_none", "126:9a9a00cc6461f20e"),
    ("d4_h0_o0.35_s1_size", "261:cbdad0e017a06b42"),
    ("d4_h0_o1_s1_size", "261:9b9f3948f05d04d6"),
    ("d4_h0_o0.35_s1_size+flow", "261:c350150282650924"),
    ("d4_h0_o1_s1_size+flow", "261:21faf048cd69a52b"),
    ("d4_h0.5_o0.35_s0.05_none", "2002:5bc32e5fff528a3a"),
    ("d4_h0.5_o1_s0.05_none", "2002:40162c9d392fb86e"),
    ("d4_h0.5_o0.35_s0.05_size", "2002:e138930911ff1b5e"),
    ("d4_h0.5_o1_s0.05_size", "2002:1325c62e1d7deced"),
    ("d4_h0.5_o0.35_s0.05_size+flow", "2002:a15e96bd45b179f0"),
    ("d4_h0.5_o1_s0.05_size+flow", "2002:7d0c77bd7ceffdbc"),
    ("d4_h0.5_o0.35_s0.25_none", "501:b41a68bf6fd54721"),
    ("d4_h0.5_o1_s0.25_none", "501:034759aed376585e"),
    ("d4_h0.5_o0.35_s0.25_size", "1039:354fb9a156479beb"),
    ("d4_h0.5_o1_s0.25_size", "1039:4e8e1e9588312403"),
    ("d4_h0.5_o0.35_s0.25_size+flow", "1039:7c671e083e320651"),
    ("d4_h0.5_o1_s0.25_size+flow", "1039:14af71439c4b4d81"),
    ("d4_h0.5_o0.35_s1_none", "126:1fe89639371b328c"),
    ("d4_h0.5_o1_s1_none", "126:db203ab8123c5800"),
    ("d4_h0.5_o0.35_s1_size", "261:8478ac839b27ca89"),
    ("d4_h0.5_o1_s1_size", "261:6dad9b3c9fb02c06"),
    ("d4_h0.5_o0.35_s1_size+flow", "261:2bbc781216214744"),
    ("d4_h0.5_o1_s1_size+flow", "261:13a07db22ff294c3"),
    ("d4_h1_o0.35_s0.05_none", "2002:fb64e144837be252"),
    ("d4_h1_o1_s0.05_none", "2002:3bc1a7863482d6e3"),
    ("d4_h1_o0.35_s0.05_size", "2002:0d7a7a71466d04ec"),
    ("d4_h1_o1_s0.05_size", "2002:1edb50b57a0a9f08"),
    ("d4_h1_o0.35_s0.05_size+flow", "2002:1553bcb6f7a452d6"),
    ("d4_h1_o1_s0.05_size+flow", "2002:bd443fee6ae5bbbf"),
    ("d4_h1_o0.35_s0.25_none", "501:b2367681daf1a782"),
    ("d4_h1_o1_s0.25_none", "501:21ec627881dfd7c8"),
    ("d4_h1_o0.35_s0.25_size", "1039:36d3862af5fbd71b"),
    ("d4_h1_o1_s0.25_size", "1039:4b77e4eb2863feaa"),
    ("d4_h1_o0.35_s0.25_size+flow", "1039:b7b67fd7424d2925"),
    ("d4_h1_o1_s0.25_size+flow", "1039:413fa5948214eee9"),
    ("d4_h1_o0.35_s1_none", "126:25fb7162e999bdbf"),
    ("d4_h1_o1_s1_none", "126:4cb984ffbe592ff2"),
    ("d4_h1_o0.35_s1_size", "261:f76624d3d29de81c"),
    ("d4_h1_o1_s1_size", "261:3b02acffdc06daee"),
    ("d4_h1_o0.35_s1_size+flow", "261:88414aa9d86d3f3b"),
    ("d4_h1_o1_s1_size+flow", "261:6b935398c21d0182"),
    ("d32_h0_o0.35_s0.05_none", "314:0274b0b7459d2727"),
    ("d32_h0_o1_s0.05_none", "314:bf5e3226e0cb35f1"),
    ("d32_h0_o0.35_s0.05_size", "672:87e3f5f648c309c4"),
    ("d32_h0_o1_s0.05_size", "672:e405a19c34e05814"),
    ("d32_h0_o0.35_s0.05_size+flow", "672:a9f610b77288733e"),
    ("d32_h0_o1_s0.05_size+flow", "672:dfdce6adbe130a3e"),
    ("d32_h0_o0.35_s0.25_none", "64:5d1ff1dcf6e16644"),
    ("d32_h0_o1_s0.25_none", "64:1d08989b8d823a95"),
    ("d32_h0_o0.35_s0.25_size", "138:cd6d3be34fff3766"),
    ("d32_h0_o1_s0.25_size", "138:92f8748866f78f36"),
    ("d32_h0_o0.35_s0.25_size+flow", "138:451f38b0576edcc9"),
    ("d32_h0_o1_s0.25_size+flow", "138:045d8dfc1edc88ab"),
    ("d32_h0_o0.35_s1_none", "17:003d868280b9fd39"),
    ("d32_h0_o1_s1_none", "17:6c36544d366d9f1d"),
    ("d32_h0_o0.35_s1_size", "36:5a65096eb00603b9"),
    ("d32_h0_o1_s1_size", "36:9915514390a30353"),
    ("d32_h0_o0.35_s1_size+flow", "36:385788c96fd1da64"),
    ("d32_h0_o1_s1_size+flow", "36:33a571a8be719970"),
    ("d32_h0.5_o0.35_s0.05_none", "314:82e0a6a427f42493"),
    ("d32_h0.5_o1_s0.05_none", "314:e1f712ee1c5e1de0"),
    ("d32_h0.5_o0.35_s0.05_size", "672:d18c2be4ac4ad957"),
    ("d32_h0.5_o1_s0.05_size", "672:0f98fa60ff994984"),
    ("d32_h0.5_o0.35_s0.05_size+flow", "672:b0b923b442503747"),
    ("d32_h0.5_o1_s0.05_size+flow", "672:e7091c468480c4c2"),
    ("d32_h0.5_o0.35_s0.25_none", "64:6da0e5b70b27f2d6"),
    ("d32_h0.5_o1_s0.25_none", "64:dcceb59059a99b9b"),
    ("d32_h0.5_o0.35_s0.25_size", "138:8c905e3f11b07b8c"),
    ("d32_h0.5_o1_s0.25_size", "138:7c416e615aeab80d"),
    ("d32_h0.5_o0.35_s0.25_size+flow", "138:cb234e59bb8916bc"),
    ("d32_h0.5_o1_s0.25_size+flow", "138:cdb9996489f878f7"),
    ("d32_h0.5_o0.35_s1_none", "17:a36a064ece2c1c26"),
    ("d32_h0.5_o1_s1_none", "17:e680f599b14f726c"),
    ("d32_h0.5_o0.35_s1_size", "36:1ae8c684c946c4bc"),
    ("d32_h0.5_o1_s1_size", "36:394020e245aa3f6c"),
    ("d32_h0.5_o0.35_s1_size+flow", "36:0dd8ca94571a163f"),
    ("d32_h0.5_o1_s1_size+flow", "36:aba5ccc728e6319c"),
    ("d32_h1_o0.35_s0.05_none", "314:c1250afe1e2b04e1"),
    ("d32_h1_o1_s0.05_none", "314:c4ad96347e34d7a2"),
    ("d32_h1_o0.35_s0.05_size", "672:baf9d918b6383657"),
    ("d32_h1_o1_s0.05_size", "672:bf3a2ec2bcdfbe1d"),
    ("d32_h1_o0.35_s0.05_size+flow", "672:1637657d3c5c1152"),
    ("d32_h1_o1_s0.05_size+flow", "672:48be7fc7af93d12a"),
    ("d32_h1_o0.35_s0.25_none", "64:17b129ed889dc215"),
    ("d32_h1_o1_s0.25_none", "64:1e9d0fd9164f485d"),
    ("d32_h1_o0.35_s0.25_size", "138:4c1b0365e0704c6b"),
    ("d32_h1_o1_s0.25_size", "138:b731dc1c6108e83d"),
    ("d32_h1_o0.35_s0.25_size+flow", "138:65f7af0c7b6f369e"),
    ("d32_h1_o1_s0.25_size+flow", "138:16840c6146d73ca2"),
    ("d32_h1_o0.35_s1_none", "17:85868e4b85620775"),
    ("d32_h1_o1_s1_none", "17:79556ac0c51d564f"),
    ("d32_h1_o0.35_s1_size", "36:ef18565262b2cd3c"),
    ("d32_h1_o1_s1_size", "36:c16274dd499f3678"),
    ("d32_h1_o0.35_s1_size+flow", "36:e7294b2d5416467b"),
    ("d32_h1_o1_s1_size+flow", "36:281a88c4b5b85b43"),
    ("d150_h0_o0.35_s0.05_none", "68:06f0023f68098de9"),
    ("d150_h0_o1_s0.05_none", "68:3e215f7abd6ec38a"),
    ("d150_h0_o0.35_s0.05_size", "145:57555e8054343d12"),
    ("d150_h0_o1_s0.05_size", "145:cc2317811966f88d"),
    ("d150_h0_o0.35_s0.05_size+flow", "145:448e8ecec4372583"),
    ("d150_h0_o1_s0.05_size+flow", "145:ddd810d8f46ef50b"),
    ("d150_h0_o0.35_s0.25_none", "14:3de91d5220228dfd"),
    ("d150_h0_o1_s0.25_none", "14:c32db26a6f286ec1"),
    ("d150_h0_o0.35_s0.25_size", "31:85c681c68d8057c0"),
    ("d150_h0_o1_s0.25_size", "31:630d9933a5a14dbe"),
    ("d150_h0_o0.35_s0.25_size+flow", "31:dc6dbfa668cd9c54"),
    ("d150_h0_o1_s0.25_size+flow", "31:99d07ab11f536daa"),
    ("d150_h0_o0.35_s1_none", "4:eba72bf8d2a2f831"),
    ("d150_h0_o1_s1_none", "4:2eb896bf04a425bb"),
    ("d150_h0_o0.35_s1_size", "9:fc01ab7b2cb75c41"),
    ("d150_h0_o1_s1_size", "9:96149a171addaa87"),
    ("d150_h0_o0.35_s1_size+flow", "9:41f247d9b241f17a"),
    ("d150_h0_o1_s1_size+flow", "9:8f69692e7e75c5a2"),
    ("d150_h0.5_o0.35_s0.05_none", "68:902c1553883aff05"),
    ("d150_h0.5_o1_s0.05_none", "68:0a29d01222aba8da"),
    ("d150_h0.5_o0.35_s0.05_size", "145:8440093935f6cf3a"),
    ("d150_h0.5_o1_s0.05_size", "145:447c52294b858a9b"),
    ("d150_h0.5_o0.35_s0.05_size+flow", "145:491e14aa02bfc187"),
    ("d150_h0.5_o1_s0.05_size+flow", "145:21622a8a3a969ef2"),
    ("d150_h0.5_o0.35_s0.25_none", "14:5d7833147762ff26"),
    ("d150_h0.5_o1_s0.25_none", "14:dd355ca5e92ca9fe"),
    ("d150_h0.5_o0.35_s0.25_size", "31:4a9060fec8be4b6c"),
    ("d150_h0.5_o1_s0.25_size", "31:ee8256f26ba464d1"),
    ("d150_h0.5_o0.35_s0.25_size+flow", "31:9d01b47df1453f7f"),
    ("d150_h0.5_o1_s0.25_size+flow", "31:69a61374ad528a0c"),
    ("d150_h0.5_o0.35_s1_none", "4:413f73759d034c6e"),
    ("d150_h0.5_o1_s1_none", "4:f702b6ee1111413b"),
    ("d150_h0.5_o0.35_s1_size", "9:9aaee09e980d77ce"),
    ("d150_h0.5_o1_s1_size", "9:1cf36286317fac78"),
    ("d150_h0.5_o0.35_s1_size+flow", "9:6f81b4ba47cd72e1"),
    ("d150_h0.5_o1_s1_size+flow", "9:189edbe11eb3a5f0"),
    ("d150_h1_o0.35_s0.05_none", "68:24e86bca03cab839"),
    ("d150_h1_o1_s0.05_none", "68:162a8877730a7348"),
    ("d150_h1_o0.35_s0.05_size", "145:0ecbe13ab3b1f089"),
    ("d150_h1_o1_s0.05_size", "145:506c10b080ea5d9f"),
    ("d150_h1_o0.35_s0.05_size+flow", "145:69165322156c7b4f"),
    ("d150_h1_o1_s0.05_size+flow", "145:0fb2d0725b9d4c19"),
    ("d150_h1_o0.35_s0.25_none", "14:f02080d1a988468a"),
    ("d150_h1_o1_s0.25_none", "14:cd799a67dc031675"),
    ("d150_h1_o0.35_s0.25_size", "31:e3fd54abd45d3551"),
    ("d150_h1_o1_s0.25_size", "31:83a4980878212f09"),
    ("d150_h1_o0.35_s0.25_size+flow", "31:8ac97d2dc5ec803b"),
    ("d150_h1_o1_s0.25_size+flow", "31:9f5c463f5217827b"),
    ("d150_h1_o0.35_s1_none", "4:f229e60e5fdc546f"),
    ("d150_h1_o1_s1_none", "4:5b62f7e7e4654fb0"),
    ("d150_h1_o0.35_s1_size", "9:ce58de636ba89502"),
    ("d150_h1_o1_s1_size", "9:aea0e4da00e4925b"),
    ("d150_h1_o0.35_s1_size+flow", "9:65b800047db4dfd2"),
    ("d150_h1_o1_s1_size+flow", "9:788ea6281540a78d"),
];
