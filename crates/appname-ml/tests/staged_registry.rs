// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Staged registry cross-check: `packs/packs.json` must parse, pass
//! discipline validation, and carry EXACTLY the pins-file values for the two
//! staged packs (a drift tripwire — the expected constants below are quoted
//! from `research/pins/models.v5.6-candidate.yaml`, the exclusive feed; if
//! this test and the registry disagree, someone re-derived a pin).
//!
//! Spec: plan §1 rule 8; D-49; ai-model-pack.v1.

use appname_ml::manifest::{DistributionMode, PackRegistry, Sha256Pin, Task};
use appname_ml::runtime::{slot_binding, SlotBinding, SlotId};
use appname_ml::verify::{load_state, pack_loadable, LoadState, PackRefusal};

/// Pins quoted verbatim from models.v5.6-candidate.yaml (B3, 2026-08-11).
const BIREFNET_SRC_SHA256: &str =
    "9ab37426bf4de0567af6b5d21b16151357149139362e6e8992021b8ce356a154";
const BIREFNET_REVISION: &str = "e2bf8e4460fc8fa32bba5ea4d94b3233d367b0e4";
const MODNET_SRC_SHA256: &str = "7c22235f0925deba15d4d63e53afcb654c47055bbcd98f56e393ab2584007ed8";
const MODNET_REVISION: &str = "28165a451e4610c9d77cfdf925a94610bb2810fb";

fn staged() -> PackRegistry {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../packs/packs.json");
    let json = std::fs::read_to_string(path).expect("packs/packs.json readable");
    PackRegistry::parse(&json).expect("staged registry parses + validates")
}

#[test]
fn staged_registry_matches_pins() {
    let reg = staged();
    assert_eq!(reg.packs.len(), 2, "exactly the two pinnable first packs");

    let b = reg.pack("bgremove-birefnet").expect("birefnet staged");
    assert_eq!(b.task, Task::BackgroundRemoval);
    assert_eq!(b.version, "1.1");
    assert_eq!(b.source.revision, BIREFNET_REVISION);
    assert_eq!(b.source.files.len(), 1);
    assert_eq!(
        b.source.files[0].bytes,
        appname_ml::manifest::PendingOr::Value(444473596)
    );
    assert_eq!(
        b.source.files[0].sha256,
        Sha256Pin::Hex(BIREFNET_SRC_SHA256.into())
    );
    assert_eq!(b.distribution.mode, DistributionMode::BundledOptionalPack);
    assert!(!b.revocable);

    let m = reg.pack("matting-modnet").expect("modnet staged");
    assert_eq!(m.task, Task::Matting);
    assert_eq!(m.version, "1.1");
    assert_eq!(m.source.revision, MODNET_REVISION);
    assert_eq!(
        m.source.files[0].bytes,
        appname_ml::manifest::PendingOr::Value(26255603)
    );
    assert_eq!(
        m.source.files[0].sha256,
        Sha256Pin::Hex(MODNET_SRC_SHA256.into())
    );
    assert_eq!(m.source.weights_license, "Apache-2.0 (same repo)");
}

#[test]
fn staged_packs_are_honestly_not_loadable_yet() {
    // Both converted artifacts are pending-packaging in the pins feed; the
    // loader must refuse typed, with no filesystem involvement.
    let reg = staged();
    for pack in &reg.packs {
        assert!(!pack_loadable(pack), "{} must not be loadable", pack.id);
        for f in &pack.converted.files {
            match load_state(pack, f) {
                LoadState::Refused(PackRefusal::ConversionUnpinned { pack_id, path }) => {
                    assert_eq!(pack_id, pack.id);
                    assert_eq!(path, f.path);
                }
                other => panic!("expected ConversionUnpinned for {}, got {other:?}", pack.id),
            }
        }
    }
}

#[test]
fn slot_bindings_point_at_staged_packs() {
    let reg = staged();
    for slot in SlotId::ALL {
        match slot_binding(slot) {
            SlotBinding::Pack { pack_id } => {
                assert!(
                    reg.pack(pack_id).is_some(),
                    "slot {} bound to unstaged pack {pack_id}",
                    slot.as_str()
                );
            }
            SlotBinding::NoPinnedPack { .. } => {
                // The recorded u2net/dis gap — bindings stay typed-unbound.
                assert!(
                    matches!(
                        slot,
                        SlotId::SubjectSelectSalient | SlotId::SubjectSelectDichotomous
                    ),
                    "unexpected unbound slot {}",
                    slot.as_str()
                );
            }
        }
    }
}

#[test]
fn license_display_surfaces_are_complete() {
    // Part 53 §53.9 / D-49: license + provenance shown before download.
    let reg = staged();
    for pack in &reg.packs {
        let d = pack.license_display();
        assert!(!d.code_license.is_empty());
        assert!(!d.weights_license.is_empty());
        assert!(!d.license_url.is_empty());
        assert!(!d.upstream_repo.is_empty());
        assert!(!d.upstream_revision.is_empty());
        // Neither first pack is CC-BY-family; the flag is for GeoCalib later.
        assert!(!d.attribution_required_in_ui);
    }
}
