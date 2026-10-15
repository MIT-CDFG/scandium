// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The D-ML-1 pipeline, end to end (CP-1.0 slice 2): a synthetic "model
//! output" (fixture mask standing in for a real inference) is sealed
//! (`CapturedModelOutput::seal`), encoded as an `appname.mask.v1` asset
//! (`MaskAssetCapture`), ingested through the door's SANCTIONED engine-side
//! ingress (`Engine::add_mask_asset`), and applied to the selection surface
//! and a layer raster mask through core's §13.2 combine doors — with the
//! inference slot still stub-refusing (typed), and the door's UNSANCTIONED
//! wire arms (`selection.set` `source:"raster"`, `mask.add` `from:"asset"`,
//! both on the fold-in register) pinned as stage-5 refusals.
//!
//! Spec: D-48 (value capture; replay never re-runs a model), ALGORITHMS.md
//! §9 / §1.4.4 (class N), ES §13.1/§13.2; the D-ML-1 door (core door slice
//! 2). DEV-dependency posture: this test binary links `appname-core`; the
//! `appname-ml` LIBRARY does not (Cargo.toml note), and core has no path to
//! appname-ml — replay remains structurally model-free.
//!
//! The probe pattern (a scripted [`KindImpl`] over the registered carrier
//! kind `selection.all`, steps keyed by envelope id so replay re-resolves
//! identically) is core's own pinned D-ML-1 test rig
//! (`mask_asset_ingress_applies_through_the_combine_doors`) — a deliberate
//! partial assembly in the documented per-crate-rig sense, not a wire
//! spelling: no shipped surface routes any envelope to these doors today.

use std::sync::Arc;

use appname_core::engine::assets::{asset_id_for, decode_mask_plane, encode_mask_plane};
use appname_core::engine::error::EngineError;
use appname_core::engine::error::ErrorCode;
use appname_core::engine::kind_impl::{EngineAssembly, KindContext, KindImpl, KindOutcome};
use appname_core::engine::mask_ops::SelectOp;
use appname_core::geom::{IRect, IVec2};
use serde_json::json;
use uuid::Uuid;

use appname_ml::capture::CapturedModelOutput;
use appname_ml::manifest::PackRegistry;
use appname_ml::pipeline::MaskAssetCapture;
use appname_ml::runtime::{
    compiled_backend, BackendId, ExecutionProvider, MlError, RasterFormat, RasterPayload, SlotId,
};
use appname_ml::sha256;
use appname_ml::verify::load_verified;

/// The synthetic "model output": a deterministic 64×64 f32 matte (stands in
/// for a real subject-select/matte inference; the runtime slot is typed-NI —
/// see `inference_slot_still_refuses_typed`).
fn fixture_matte() -> (Vec<f32>, RasterPayload) {
    let values: Vec<f32> = (0..64 * 64)
        .map(|i| {
            let (x, y) = (i % 64, i / 64);
            (((x + y) % 33) as f32 / 32.0).min(1.0)
        })
        .collect();
    let payload = RasterPayload {
        width: 64,
        height: 64,
        format: RasterFormat::MaskF32Le,
        data: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
    };
    (values, payload)
}

/// A sealed capture through the REAL load gate (fixture pack; no bypass
/// constructor exists on this path).
fn sealed_fixture(output: RasterPayload, tag: &str) -> CapturedModelOutput {
    let model_bytes = b"door-pipeline-fixture-model-bytes";
    let sha = sha256::hex_digest(model_bytes);
    let manifest = format!(
        r#"{{"packs":[{{"id":"door-fixture","name":"Door fixture","version":"1.4","task":"matting",
"source":{{"repo":"https://example.invalid/r","revision":"0","files":[{{"name":"w","bytes":33,"sha256":"{src}"}}],
"code_license":"Apache-2.0","weights_license":"Apache-2.0","license_url":"https://example.invalid/L"}},
"converted":{{"recipe":"none — upstream artifact used directly","files":[{{"path":"m/w.onnx","sha256":"{sha}"}}]}},
"distribution":{{"mode":"bundled-optional-pack","redistribution_allowed":true}},
"legal":{{"rights_ref":"test"}},"revocable":false}}]}}"#,
        src = "a".repeat(64),
        sha = sha,
    );
    let reg = PackRegistry::parse(&manifest).unwrap();
    let root = std::env::temp_dir().join(format!(
        "appname-ml-door-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("m")).unwrap();
    std::fs::write(root.join("m/w.onnx"), model_bytes).unwrap();
    let pack = reg.pack("door-fixture").unwrap();
    let art = load_verified(pack, &pack.converted.files[0], &root).unwrap();
    let cap = CapturedModelOutput::seal(
        SlotId::SubjectSelectHighQuality,
        &art,
        BackendId("test-backend"),
        ExecutionProvider::Cpu,
        "blake3:fixture-input".into(),
        output,
    );
    let _ = std::fs::remove_dir_all(&root);
    cap
}

/// Core's own D-ML-1 probe rig: a scripted step per envelope id over the
/// registered carrier kind `selection.all`, so undo/redo/replay re-resolve
/// identically (assets are captured values).
struct Step {
    asset: Uuid,
    op: SelectOp,
    mask: Option<appname_core::graph::id::MaskId>,
}

struct MlProbe {
    steps: std::collections::HashMap<Uuid, Step>,
}

impl KindImpl for MlProbe {
    fn kind(&self) -> &'static str {
        "selection.all"
    }
    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let step = self
            .steps
            .get(&cx.envelope().id)
            .ok_or_else(|| EngineError::bad_target("probe: unscripted envelope"))?;
        match step.mask {
            None => cx.apply_mask_asset_to_selection(step.asset, step.op)?,
            Some(m) => cx.apply_mask_asset_to_layer_mask(m, step.asset, step.op)?,
        }
        Ok(KindOutcome::default())
    }
}

fn e_id(n: u32) -> Uuid {
    Uuid::parse_str(&format!("d0d0a100-0000-4000-8000-0000{n:08x}")).unwrap()
}

fn apply(
    e: &mut appname_core::engine::Engine,
    n: u32,
    kind: &str,
    params: serde_json::Value,
    target: Option<Uuid>,
) -> Result<appname_core::engine::ApplyReceipt, EngineError> {
    let mut v = json!({"schema": "appname.command.v1", "id": e_id(n).to_string(),
                       "kind": kind, "params": params});
    if let Some(t) = target {
        v["target"] = json!({"layerId": t.to_string()});
    }
    e.apply_json(&v.to_string())
}

/// Cross-crate wire parity: appname-ml's `appname.mask.v1` encoding is
/// byte-identical to core's, decodes bit-exactly, and lands on the same
/// content-derived asset id.
#[test]
fn wire_parity_with_core() {
    let (values, payload) = fixture_matte();
    let mac = MaskAssetCapture::from_capture(sealed_fixture(payload, "parity"), 10, 6).unwrap();
    let rect = IRect::new(10, 6, 74, 70);

    let core_bytes = encode_mask_plane(rect, &values);
    assert_eq!(
        mac.asset_bytes(),
        core_bytes.as_slice(),
        "ml-side encoding must mirror core byte-for-byte"
    );
    let (r2, v2) = decode_mask_plane(mac.asset_bytes()).unwrap();
    assert_eq!(r2, rect);
    assert_eq!(
        v2.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        values.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        "bit-exact through the door's decoder"
    );
    assert_eq!(
        asset_id_for(mac.asset_bytes()),
        asset_id_for(&core_bytes),
        "same content-derived asset id on both sides"
    );
    assert_eq!(mac.asset_sha256(), sha256::hex_digest(&core_bytes));
}

/// THE pipeline: seal → mask.v1 asset → sanctioned ingress → §13.2 door →
/// undo/redo byte identity → replay determinism (no model anywhere near the
/// replay; the asset is the captured value).
#[test]
fn capture_to_door_pipeline() {
    let (values, payload) = fixture_matte();
    let mac = MaskAssetCapture::from_capture(sealed_fixture(payload, "pipeline"), 10, 6).unwrap();
    let expected_id = asset_id_for(mac.asset_bytes());
    let mid = appname_core::graph::id::MaskId::from_uuid(Uuid::from_u128(0x4d61));

    let mut steps = std::collections::HashMap::new();
    steps.insert(
        e_id(3),
        Step {
            asset: expected_id,
            op: SelectOp::Replace,
            mask: None,
        },
    );
    steps.insert(
        e_id(4),
        Step {
            asset: expected_id,
            op: SelectOp::Replace,
            mask: Some(mid),
        },
    );
    let assembly = appname_core::engine::kinds::register(
        EngineAssembly::new().with_kind(Arc::new(MlProbe { steps })),
    );
    let mut e = assembly.build();
    apply(
        &mut e,
        1,
        "doc.new",
        json!({"width": 128, "height": 128, "background": "white"}),
        None,
    )
    .unwrap();
    let bg = e.document().unwrap().z_order_walk()[0].as_uuid();
    apply(
        &mut e,
        2,
        "mask.add",
        json!({"kind": "raster", "from": "hide_all", "newMaskId": mid.as_uuid().to_string()}),
        Some(bg),
    )
    .unwrap();

    // Sanctioned ingress: the engine validates and derives the content id.
    let ingested = e.add_mask_asset(mac.asset_bytes().to_vec()).unwrap();
    assert_eq!(ingested, expected_id, "content-derived id, precomputable");

    // Door (a): capture → selection surface, Replace.
    let pre = e.state_hash();
    apply(&mut e, 3, "selection.all", json!({}), None).unwrap();
    {
        let d = e.document().unwrap();
        assert!(d.selection().active());
        assert_eq!(d.selection().bounds(), IRect::new(10, 6, 74, 70));
        // Readback == the sealed model output, placed at the origin:
        // document (30, 26) is plane (20, 20) → value ((20+20)%33)/32.
        assert_eq!(
            d.selection().mask().read_pixel_f32(IVec2::new(30, 26))[0],
            values[20 * 64 + 20]
        );
        assert_eq!(
            d.selection().mask().read_pixel_f32(IVec2::new(5, 5))[0],
            0.0,
            "outside the plane stays clear"
        );
    }

    // Door (b): the same captured asset onto a layer raster mask.
    apply(&mut e, 4, "selection.all", json!({}), None).unwrap();
    {
        let d = e.document().unwrap();
        let m = d.mask(mid).unwrap();
        assert_eq!(m.coverage().bounds(), IRect::new(10, 6, 74, 70));
        assert_eq!(
            m.coverage().read_pixel_f32(IVec2::new(30, 26))[0],
            values[20 * 64 + 20]
        );
    }
    let after = e.state_hash();

    // Undo both applications; redo restores byte-exact (§10.4 deltas).
    e.undo().unwrap();
    e.undo().unwrap();
    assert_eq!(e.state_hash(), pre);
    e.redo().unwrap();
    e.redo().unwrap();
    assert_eq!(e.state_hash(), after);

    // Replay determinism: the journal + captured asset reproduce the state
    // byte-identically — no model ran, none is installed, none CAN run
    // (the runtime is typed-NI; next test). D-48 closed end to end.
    let replayed = e.replay_assembled(e.journal()).unwrap();
    assert_eq!(replayed.state_hash(), after);
}

/// The runtime posture stands while the pipeline is real: session
/// acquisition is a typed refusal (RuntimeNotCompiled without `ml-models`;
/// RuntimeNotLinked with it, until the ort record consumes) — the fixture
/// mask is the ONLY way this test suite ever obtained a "model output".
#[test]
fn inference_slot_still_refuses_typed() {
    #[cfg(not(feature = "ml-models"))]
    match compiled_backend() {
        Err(e) => assert_eq!(e, MlError::RuntimeNotCompiled),
        Ok(_) => panic!("no backend may exist in the default build"),
    }
    #[cfg(feature = "ml-models")]
    {
        let backend = compiled_backend().expect("stub backend compiled");
        let (_, payload) = fixture_matte();
        // Reuse the load-gate fixture to hold a real VerifiedArtifact.
        let cap = sealed_fixture(payload, "stub");
        assert_eq!(cap.backend, BackendId("test-backend"));
        // The stub cannot mint sessions: typed RuntimeNotLinked.
        let manifest_err = {
            let model_bytes = b"door-pipeline-fixture-model-bytes";
            let sha = sha256::hex_digest(model_bytes);
            let manifest = format!(
                r#"{{"packs":[{{"id":"door-fixture","name":"D","version":"1.4","task":"matting",
"source":{{"repo":"https://example.invalid/r","revision":"0","files":[{{"name":"w","bytes":33,"sha256":"{src}"}}],
"code_license":"Apache-2.0","weights_license":"Apache-2.0","license_url":"https://example.invalid/L"}},
"converted":{{"recipe":"none — upstream artifact used directly","files":[{{"path":"m/w.onnx","sha256":"{sha}"}}]}},
"distribution":{{"mode":"bundled-optional-pack","redistribution_allowed":true}},
"legal":{{"rights_ref":"test"}},"revocable":false}}]}}"#,
                src = "a".repeat(64),
                sha = sha,
            );
            let reg = PackRegistry::parse(&manifest).unwrap();
            let root = std::env::temp_dir().join(format!(
                "appname-ml-door-stubsess-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("m")).unwrap();
            std::fs::write(root.join("m/w.onnx"), model_bytes).unwrap();
            let pack = reg.pack("door-fixture").unwrap();
            let art = load_verified(pack, &pack.converted.files[0], &root).unwrap();
            let out = backend.create_session(&art, ExecutionProvider::Cpu);
            let _ = std::fs::remove_dir_all(&root);
            out.err()
        };
        assert!(
            matches!(manifest_err, Some(MlError::RuntimeNotLinked { .. })),
            "{manifest_err:?}"
        );
    }
}

/// Wire-arm posture (§3.13 check): the door's wire spellings are
/// UNSANCTIONED at v5.5 and on the fold-in register — the engine's stage-5
/// validation refuses them TODAY, through THE production assembly (the
/// single-source constructor every shipped surface uses), and nothing in
/// appname-ml emits them (the library builds no envelopes at all). Pinned
/// so a silent fold-in arm cannot land unnoticed from this side.
#[test]
fn unsanctioned_wire_spellings_refuse_typed() {
    let mut e = appname_assembly::production_assembly().build();
    apply(
        &mut e,
        1,
        "doc.new",
        json!({"width": 32, "height": 32, "background": "white"}),
        None,
    )
    .unwrap();
    let bg = e.document().unwrap().z_order_walk()[0].as_uuid();
    let pre = e.state_hash();

    // selection.set source:"raster" — not in the v5.5 source enum.
    let err = apply(
        &mut e,
        2,
        "selection.set",
        json!({"source": "raster", "assetId": Uuid::from_u128(7).to_string()}),
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange, "{err:?}");

    // mask.add from:"asset" — not in the v5.5 from enum.
    let err = apply(
        &mut e,
        3,
        "mask.add",
        json!({"kind": "raster", "from": "asset", "newMaskId": Uuid::from_u128(8).to_string()}),
        Some(bg),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange, "{err:?}");

    assert_eq!(e.state_hash(), pre, "refused spellings mutate nothing");
}
