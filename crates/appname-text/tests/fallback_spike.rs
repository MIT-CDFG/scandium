// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §44.2 fallback-chain acceptance (CP-1.0 slice C-2): the resolution
//! ORDER, whole-cluster coverage integrity, PostScript-name dedup, and
//! the §44.8 cluster-count invariant — all over procedural subset
//! faces (different cmaps, no vendored font files).

mod common;

use appname_text::fontdb::{DbProbe, FaceInfo, FaceSource, FontDb, ResolveCtx, TOFU_FACE};
use appname_text::pipeline::GraphemeSegmenter;
use appname_text::pipeline::{itemize, FaceId, RunSpan, StyleId};
use appname_text::providers::{IcuGraphemeSegmenter, IcuScriptClassifier, UnicodeBidiProvider};

fn info(family: &str, ps: &str, source: FaceSource) -> FaceInfo {
    FaceInfo {
        family: family.into(),
        style: "Regular".into(),
        postscript_name: ps.into(),
        source,
        cjk_variant: None,
    }
}

/// A db with: run face = Latin subset, embedded Hebrew subset, a CJK
/// system face, and a full bundled face.
fn spike_db() -> (FontDb, ResolveCtx) {
    let mut db = FontDb::new();
    let latin: Vec<char> = vec![' ', '!', 'A', 'B', 'C', 'D', 'E', 'F'];
    db.register(
        FaceId(1),
        &common::build_subset(&latin),
        info("SpikeLatin", "SpikeLatin-Regular", FaceSource::User),
    )
    .unwrap();
    db.register(
        FaceId(2),
        &common::build_subset(&[' ', '\u{05D0}', '\u{05D1}']),
        info(
            "SpikeHebrew",
            "SpikeHebrew-Regular",
            FaceSource::DocEmbedded,
        ),
    )
    .unwrap();
    db.register(
        FaceId(3),
        &common::build_subset(&[' ', '\u{6C38}']),
        info("SpikeCJK", "SpikeCJK-Regular", FaceSource::System),
    )
    .unwrap();
    db.register(
        FaceId(4),
        &common::build(),
        info("SpikeFull", "SpikeFull-Regular", FaceSource::Bundled),
    )
    .unwrap();
    let ctx = ResolveCtx {
        run_face: Some(FaceId(1)),
        embedded: vec![FaceId(2)],
        ..Default::default()
    };
    (db, ctx)
}

#[test]
fn run_face_wins_when_it_covers() {
    let (db, ctx) = spike_db();
    assert_eq!(db.resolve(&ctx, *b"Latn", "A"), FaceId(1));
    assert_eq!(db.resolve(&ctx, *b"Latn", "F"), FaceId(1));
}

#[test]
fn embedded_faces_probe_before_preferences_and_index() {
    let (db, ctx) = spike_db();
    // Hebrew: run face misses → step 2 embedded face covers.
    assert_eq!(db.resolve(&ctx, *b"Hebr", "\u{05D0}"), FaceId(2));
}

#[test]
fn script_preference_chain_probes_at_step_three() {
    let (mut db, mut ctx) = spike_db();
    // A second CJK-capable face registered as a preference target.
    db.register(
        FaceId(9),
        &common::build_subset(&[' ', '\u{6C38}']),
        info("PrefHan", "PrefHan-Regular", FaceSource::User),
    )
    .unwrap();
    ctx.script_prefs.insert(*b"Hani", vec![FaceId(9)]);
    // With the pref: step 3 resolves before enumeration would pick
    // SpikeCJK/SpikeFull.
    assert_eq!(db.resolve(&ctx, *b"Hani", "\u{6C38}"), FaceId(9));
    // Without a pref for the script: enumeration decides (bundled
    // SpikeFull probes at step 4 before system SpikeCJK at step 5).
    let (db2, ctx2) = spike_db();
    assert_eq!(db2.resolve(&ctx2, *b"Hani", "\u{6C38}"), FaceId(4));
}

#[test]
fn enumeration_orders_by_family_then_postscript_bytes() {
    let mut db = FontDb::new();
    let cjk: Vec<char> = vec![' ', '\u{6C38}'];
    for (id, fam, ps) in [
        (FaceId(1), "ZFace", "ZFace-Regular"),
        (FaceId(2), "AFace", "AFace-Regular"),
        (FaceId(3), "AFace", "AAAA-Regular"),
    ] {
        db.register(
            id,
            &common::build_subset(&cjk),
            info(fam, ps, FaceSource::System),
        )
        .unwrap();
    }
    let ctx = ResolveCtx::default();
    // family "AFace" < "ZFace"; within AFace, PS "AAAA-…" < "AFace-…".
    assert_eq!(db.resolve(&ctx, *b"Hani", "\u{6C38}"), FaceId(3));
}

#[test]
fn duplicate_postscript_names_collapse_user_over_system_over_bundled() {
    let mut db = FontDb::new();
    let cjk: Vec<char> = vec![' ', '\u{6C38}'];
    db.register(
        FaceId(1),
        &common::build_subset(&cjk),
        info("Dup", "Dup-Regular", FaceSource::Bundled),
    )
    .unwrap();
    db.register(
        FaceId(2),
        &common::build_subset(&cjk),
        info("Dup", "Dup-Regular", FaceSource::System),
    )
    .unwrap();
    let ctx = ResolveCtx::default();
    // System shadows bundled…
    assert_eq!(db.resolve(&ctx, *b"Hani", "\u{6C38}"), FaceId(2));
    // …and user shadows system (§23.5 "a shadowed face can never win").
    db.register(
        FaceId(3),
        &common::build_subset(&cjk),
        info("Dup", "Dup-Regular", FaceSource::User),
    )
    .unwrap();
    assert_eq!(db.resolve(&ctx, *b"Hani", "\u{6C38}"), FaceId(3));
}

#[test]
fn whole_cluster_coverage_never_splits_a_cluster() {
    let (db, ctx) = spike_db();
    // "A" + COMBINING ACUTE is ONE cluster; no face covers the mark, so
    // the WHOLE cluster falls through to tofu — the base being covered
    // by four faces is irrelevant (§44.2: a mark can never come from a
    // different face than its base).
    assert!(db.covers(FaceId(1), "A"));
    assert!(!db.covers(FaceId(1), "A\u{0301}"));
    assert_eq!(db.resolve(&ctx, *b"Latn", "A\u{0301}"), TOFU_FACE);
}

#[test]
fn default_ignorables_are_coverage_exempt() {
    let (db, ctx) = spike_db();
    // ZWJ joins the cluster; no face maps it; the exemption keeps the
    // run face (ZWJ renders zero-width, never tofu).
    assert!(db.covers(FaceId(1), "A\u{200D}"));
    assert_eq!(db.resolve(&ctx, *b"Latn", "A\u{200D}"), FaceId(1));
    // Variation selector likewise.
    assert!(db.covers(FaceId(1), "A\u{FE0F}"));
}

#[test]
fn itemizer_splits_at_fallback_face_changes_and_keeps_cluster_count() {
    let (db, ctx) = spike_db();
    let scripts = IcuScriptClassifier::new();
    let probe = DbProbe {
        db: &db,
        ctx: &ctx,
        scripts: &scripts,
    };
    let text = "AB \u{05D0}\u{05D1} \u{6C38}!";
    let items = itemize(
        &[RunSpan {
            text,
            style: StyleId(1),
        }],
        &scripts,
        &UnicodeBidiProvider,
        &IcuGraphemeSegmenter::new(),
        &probe,
    );
    // Faces along the text: Latin run face → embedded Hebrew → the
    // step-4 full face for 永 → back to the run face for '!'.
    let faces: Vec<u32> = items.iter().map(|it| it.face.0).collect();
    assert!(faces.starts_with(&[1]));
    assert!(faces.contains(&2), "embedded Hebrew face used: {faces:?}");
    assert!(
        faces.contains(&4),
        "bundled full face used for CJK: {faces:?}"
    );
    // §44.8 invariant: fallback changes faces, never text structure —
    // the items exactly partition the text.
    let mut cursor = 0;
    for it in &items {
        assert_eq!(it.range.start, cursor);
        cursor = it.range.end;
    }
    assert_eq!(cursor, text.len());
    // And every boundary lands on a UAX #29 cluster boundary.
    let bounds = IcuGraphemeSegmenter::new().cluster_boundaries(text);
    for it in &items {
        assert!(bounds.contains(&it.range.start));
        assert!(bounds.contains(&it.range.end));
    }
}

#[test]
fn bundled_cjk_variant_selects_by_run_language_else_ui_locale() {
    use appname_text::fontdb::CjkVariant;
    let mut db = FontDb::new();
    let cjk: Vec<char> = vec![' ', '\u{6C38}'];
    // Four bundled CJK "variants" — same coverage, tagged variants.
    // Family names chosen so BYTE ORDER would pick JP if the language
    // rule were absent (the C-2 review's exact failure prediction).
    for (id, fam, variant) in [
        (FaceId(1), "CJK-A-JP", CjkVariant::Jp),
        (FaceId(2), "CJK-B-KR", CjkVariant::Kr),
        (FaceId(3), "CJK-C-SC", CjkVariant::Sc),
        (FaceId(4), "CJK-D-TC", CjkVariant::Tc),
    ] {
        let mut i = info(fam, fam, FaceSource::Bundled);
        i.cjk_variant = Some(variant);
        db.register(id, &common::build_subset(&cjk), i).unwrap();
    }
    // Run language wins.
    let mut ctx = ResolveCtx {
        run_language: Some("zh-Hans".into()),
        ui_locale: "en".into(),
        ..Default::default()
    };
    assert_eq!(
        db.resolve(&ctx, *b"Hani", "\u{6C38}"),
        FaceId(3),
        "SC by run language"
    );
    ctx.run_language = Some("zh-Hant-TW".into());
    assert_eq!(
        db.resolve(&ctx, *b"Hani", "\u{6C38}"),
        FaceId(4),
        "TC by script subtag"
    );
    ctx.run_language = Some("ko-KR".into());
    assert_eq!(
        db.resolve(&ctx, *b"Hani", "\u{6C38}"),
        FaceId(2),
        "KR by run language"
    );
    // No run language: UI locale decides.
    ctx.run_language = None;
    ctx.ui_locale = "ja-JP".into();
    assert_eq!(
        db.resolve(&ctx, *b"Hani", "\u{6C38}"),
        FaceId(1),
        "JP by UI locale"
    );
    // Neither is CJK: deterministic byte order (the honest-limit case).
    ctx.ui_locale = "en".into();
    assert_eq!(
        db.resolve(&ctx, *b"Hani", "\u{6C38}"),
        FaceId(1),
        "family byte order"
    );
}

#[test]
fn cjk_variant_boost_never_touches_non_cjk_clusters() {
    use appname_text::fontdb::CjkVariant;
    // The C-3 review's exact probe: an untagged bundled face that
    // sorts FIRST by family, and a Jp-tagged bundled face sorting
    // after — both covering Latin. Under a Japanese UI locale a Latin
    // cluster must resolve by the normal ordering (the untagged face),
    // never take the CJK boost.
    let mut db = FontDb::new();
    let latin: Vec<char> = vec![' ', 'A', 'B'];
    db.register(
        FaceId(1),
        &common::build_subset(&latin),
        info("Alpha", "Alpha-Regular", FaceSource::Bundled),
    )
    .unwrap();
    let mut jp = info("Zeta-JP", "Zeta-JP-Regular", FaceSource::Bundled);
    jp.cjk_variant = Some(CjkVariant::Jp);
    db.register(FaceId(2), &common::build_subset(&latin), jp)
        .unwrap();
    let ctx = ResolveCtx {
        ui_locale: "ja-JP".into(),
        ..Default::default()
    };
    assert_eq!(
        db.resolve(&ctx, *b"Latn", "A"),
        FaceId(1),
        "Latin cluster: family byte order, no variant boost"
    );
}

#[test]
fn tofu_is_reported_never_silent() {
    let (db, ctx) = spike_db();
    // Greek is in no spike face: the resolver must say TOFU, not fall
    // back to an arbitrary face (§44.2 step 6 / §23.5 "nothing is ever
    // silently invisible").
    assert_eq!(db.resolve(&ctx, *b"Grek", "\u{03B1}"), TOFU_FACE);
}

#[test]
fn resolution_is_independent_of_registration_order() {
    let build = |order: &[u32]| -> Vec<u32> {
        let mut db = FontDb::new();
        let cjk: Vec<char> = vec![' ', '\u{6C38}'];
        for &id in order {
            let (fam, ps) = match id {
                1 => ("BFace", "BFace-Regular"),
                2 => ("AFace", "AFace-Regular"),
                _ => ("CFace", "CFace-Regular"),
            };
            db.register(
                FaceId(id),
                &common::build_subset(&cjk),
                info(fam, ps, FaceSource::System),
            )
            .unwrap();
        }
        let ctx = ResolveCtx::default();
        vec![db.resolve(&ctx, *b"Hani", "\u{6C38}").0]
    };
    assert_eq!(build(&[1, 2, 3]), build(&[3, 2, 1]));
    assert_eq!(build(&[2, 3, 1]), build(&[1, 3, 2]));
}
