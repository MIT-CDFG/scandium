// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §10.1/D-48 (an assembled engine's behavior is a pure function
//! of (assembly, journal)); integration-log binding arbitration 1
//! (construction-only registration). Core door slice 2, item D — the
//! SINGLE-SOURCE ASSEMBLY CONSTRUCTOR (the structural truth-gap
//! retirement, designed for O-CORE/O-SCRIPT).
//!
//! WHY THIS CRATE EXISTS. The production register() chain was hand-copied
//! at four call sites (cli, cabi, the batch bin, the conformance
//! `assembled_engine`) plus a test mirror — and the batch bin HAD already
//! drifted once (its doc comment records the O-INT MAJOR-1 fork: missing
//! adjust/filters/vector, `unknown_op_kind` refusals through
//! `appname-actions run` for envelopes the ABI served fine). Core cannot
//! host the composed chain itself (the producer crates depend on core —
//! hosting there is a dependency cycle), so the INVERSION lands here: a
//! leaf composition crate whose ONLY job is the manifest. The property is
//! absolute: **every shipped surface calls [`production_assembly`]**;
//! adding a wrap (appname-text's coming `with_text_layout` registration,
//! a future appname-render op pack) is ONE edit to [`PRODUCTION_CHAIN`]
//! and reaches all consumers.
//!
//! TWO guards enforce the property (review MAJOR-1, ruled shape — the
//! first cut only enumerated known sites, which is not "anywhere"):
//! (1) a POSITIVE check that each shipped call site calls
//! [`production_assembly`]; (2) a WORKSPACE-WALK NEGATIVE SCAN over
//! every `crates/**` + `tests/**` `.rs` file for the chain fingerprint —
//! the qualified wrap-call tokens (`appname_vector::register(` /
//! `appname_filters::register(` / `appname_adjust::register(`, the house
//! spelling of every hand-rolled chain the tree has ever carried) —
//! outside this crate and outside an explicit in-test allowlist of
//! deliberate-partial rigs. A NEW unlisted spelling anywhere in the
//! workspace turns the test red. (Token-based: an unqualified
//! `use …::register; register(a)` alias would evade the scan — no such
//! spelling exists in the tree, and the positive check still covers the
//! shipped surfaces.)
//!
//! Deliberate partial assemblies (bare-core probes, per-crate test rigs,
//! the fuzz targets) are NOT this crate's business — they are local by
//! design and never claim production parity. A partial rig that wants to
//! stay PRODUCTION-SYNCED without the full chain folds
//! [`PRODUCTION_CHAIN`] itself, filtering the named wrap it replaces
//! (the conformance combine-op probe rig is the worked example: every
//! wrap except `appname_vector::register`, probe service attached) — no
//! fingerprint tokens, new manifest rows reach it automatically.

use appname_core::EngineAssembly;

/// One named wrap of the production chain (the registration manifest —
/// `&'static` so the chain is enumerable by tests and diagnostics).
pub type Wrap = fn(EngineAssembly) -> EngineAssembly;

/// The seed of the chain: the O-TOOLS kind assembly (stroke/transform/
/// doc/selection kinds + CAF). Everything else wraps this.
pub fn seed() -> EngineAssembly {
    appname_tools::kind_impls::assembly()
}

/// THE production registration manifest, applied in order over [`seed`].
/// Order is the historical (proven) chain: core kinds → the brokered
/// tools-fill swap → filter kinds+ops → adjust kinds+ops → the vector
/// geometry service. A NEW registration (e.g. appname-text's TextLayout
/// service at its wiring slice) is one row here — and lands at every
/// call site at once.
/// Door C4 wrap (slice 3 addendum): render's §4.5 kernel routing into
/// core's `with_blend_kernels` seam — the ONE production injection site
/// (`render_separable_kernel` 20 modes / `render_triple_kernel` 6 modes /
/// Dissolve None from both; core's apply-image arms dispatch on it, the
/// one-math-library clause). Fn-pointer identity is test-pinned below.
fn with_render_blend_kernels(assembly: EngineAssembly) -> EngineAssembly {
    assembly.with_blend_kernels(appname_core::BlendKernelRouting {
        separable: appname_render::render_separable_kernel,
        triple: appname_render::render_triple_kernel,
    })
}

/// Composite-leg fill-math wrap (the shape-serve one-math-library
/// clause; the C4 `with_render_blend_kernels` precedent): appname-text's
/// exported fill conversion into core's `with_fill_convert` seam — the
/// SHAPE composite arm converts + premultiplies its FillContent through
/// these, re-deriving no EOTF/§4.4 constants. Fn-pointer identity is
/// test-pinned below.
fn with_text_fill_convert(assembly: EngineAssembly) -> EngineAssembly {
    assembly.with_fill_convert(appname_core::FillConvertRouting {
        fill_to_linear_rgba: appname_text::fill_to_linear_rgba,
        premultiply_coverage: appname_text::premultiply_coverage,
    })
}

pub const PRODUCTION_CHAIN: &[(&str, Wrap)] = &[
    (
        "appname_core::engine::kinds::register",
        appname_core::engine::kinds::register,
    ),
    (
        "appname_tools::kind_impls::with_tools_fill",
        appname_tools::kind_impls::with_tools_fill,
    ),
    ("appname_filters::register", appname_filters::register),
    ("appname_adjust::register", appname_adjust::register),
    ("appname_vector::register", appname_vector::register),
    (
        "appname_assembly::with_render_blend_kernels",
        with_render_blend_kernels,
    ),
    // TEXTVEC Doors 6+10 (the text-kinds wiring slice): THE TextLayout
    // service — flips the five §40.7 text arms live at every call site.
    ("appname_text::register", appname_text::register),
    // Composite-leg slice: the fill-math seam — flips the SHAPE
    // reference-composite arm from typed-refusal to serving at every
    // call site (text pages compose their own fills; this row is the
    // shape leg's second door).
    (
        "appname_assembly::with_text_fill_convert",
        with_text_fill_convert,
    ),
];

/// THE one production-assembly constructor. cli, cabi, the batch bin,
/// and the conformance `assembled_engine` all call exactly this — no
/// shipped surface composes its own chain.
pub fn production_assembly() -> EngineAssembly {
    PRODUCTION_CHAIN
        .iter()
        .fold(seed(), |assembly, (_, wrap)| wrap(assembly))
}

/// The registration-manifest name of the row whose trait-object register
/// the handle variant substitutes for `register_with_handle` — the ONE
/// point of divergence between the two constructors, and it changes only
/// which coercion runs, never the chain.
const TEXT_REGISTER_ROW: &str = "appname_text::register";

/// The production assembly PLUS the retained CONCRETE `TextLayoutService`
/// handle (request-10 font door — the surface O-SCRIPT's provide-face
/// ABI consumes after this slice merges).
///
/// SHARES [`PRODUCTION_CHAIN`] with [`production_assembly`] BY
/// CONSTRUCTION: it folds the SAME manifest, substituting only the
/// [`TEXT_REGISTER_ROW`] wrap's trait-object `register` for
/// `register_with_handle` (TEXTVEC's coercion — `Arc<TextLayoutService>`
/// → `Arc<dyn TextLayout>` for the assembly, the concrete Arc handed
/// back) to capture the handle. There is NO second independent fold: any
/// row added/removed/reordered in the manifest reaches BOTH constructors
/// identically, so a shipped surface can never sit in the guard's blind
/// spot. The chain-equality pin below proves the two produce the same
/// composition.
pub fn production_assembly_with_text_handle() -> (
    EngineAssembly,
    std::sync::Arc<appname_text::TextLayoutService>,
) {
    let mut handle: Option<std::sync::Arc<appname_text::TextLayoutService>> = None;
    let assembly = PRODUCTION_CHAIN
        .iter()
        .fold(seed(), |assembly, (name, wrap)| {
            if *name == TEXT_REGISTER_ROW {
                let (a, h) = appname_text::register_with_handle(assembly);
                handle = Some(h);
                a
            } else {
                wrap(assembly)
            }
        });
    (
        assembly,
        handle.expect("PRODUCTION_CHAIN carries the appname_text::register row"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §22 fill layers, verified where engine-presence questions must be
    /// answered: on the PRODUCTION assembly, not on a per-crate test rig
    /// (a partial rig can serve a kind production never sees, and can
    /// refuse one production serves).
    ///
    /// # Disposition RE-STATED (this test replaces a tripwire)
    ///
    /// Its predecessor pinned the opposite pair — command door SERVES,
    /// composite door REFUSES — and said so explicitly, so that it would
    /// "turn red the day the composite row lands" and force the
    /// disposition to be re-stated rather than drift. The composite row
    /// has landed: `solid`, `gradient` and `none` fill content now
    /// COMPOSITE at both doors. `pattern` and `noise` still refuse typed,
    /// because §22.5 pattern assets have no definition door, schema, or
    /// AssetKind, and `$defs/gradient` carries no noise payload — nothing
    /// to resolve or synthesize from (see
    /// `appname_core::engine::fill_reg::render_fill_page`).
    ///
    /// # The tripwire did NOT actually fire, which is why it is rewritten
    ///
    /// It went on passing after fill began compositing. Its fixture piled
    /// all four arms into ONE document, so the surviving `pattern`
    /// refusal kept `export_png_bytes` returning `Err` — and its message
    /// assertion (`contains("fill")`) matched the pattern refusal's own
    /// text, `fill content "pattern" …`. A green test whose stated
    /// proposition ("a fill layer has no reference composite path yet")
    /// had become FALSE: it no longer discriminated, and would have
    /// passed for an implementation that composited fill layers
    /// completely wrongly. The structural fix is below and is the whole
    /// reason the arms are now walked ONE DOCUMENT AT A TIME: a refusal
    /// can then only be attributed to the arm under test.
    #[test]
    fn every_fill_arm_serves_or_refuses_identically_at_the_command_and_composite_doors() {
        let cmd = |id: &str, kind: &str, params: &str| {
            format!(
                r#"{{"schema":"appname.command.v1","id":"{id}","kind":"{kind}","params":{params}}}"#
            )
        };
        // ONE fill layer per document — see the doc comment.
        let doc_with = |arm: &str| {
            let mut e = production_assembly().build();
            e.apply_json(&cmd(
                "f0000000-0000-4000-8000-000000000001",
                "doc.new",
                r#"{"width":16,"height":16,"background":"white"}"#,
            ))
            .expect("doc.new serves on the production chain");
            let r = e
                .apply_json(&cmd(
                    "f0000000-0000-4000-8000-000000000010",
                    "layer.add",
                    &format!(
                        r#"{{"kind":"fill","newLayerId":"f0000000-0000-4000-8000-000000000020",
                            "kindParams":{arm}}}"#
                    ),
                ))
                .unwrap_or_else(|err| panic!("arm must serve at the COMMAND door, got {err:?}"));
            assert_eq!(r.created_ids.len(), 1, "the arm creates one node");
            e
        };

        const GRAD: &str = r#"{"type":"gradient","gradient":{"kind":"linear","stops":[
             {"pos":0,"color":{"model":"rgb","comps":[0,0,0,0]}},
             {"pos":1,"color":{"model":"rgb","comps":[1,1,1,0]}}]}}"#;

        // -- SERVED arms: both doors, no refusal ------------------------
        for (name, arm) in [
            ("none", r#"{"type":"none"}"#),
            (
                "solid",
                r#"{"type":"solid","color":{"model":"rgb","comps":[1,0,0,0]}}"#,
            ),
            ("gradient", GRAD),
        ] {
            let e = doc_with(arm);
            appname_core::io::png_io::export_png_bytes(&e).unwrap_or_else(|err| {
                panic!("arm {name} must COMPOSITE on the production chain, got {err:?}")
            });
        }

        // -- REFUSED arms: typed, and naming their own cause ------------
        for (name, arm, needle) in [
            (
                "pattern",
                r#"{"type":"pattern","patternRef":"assets/paper"}"#,
                "pattern",
            ),
            (
                "noise",
                r#"{"type":"gradient","gradient":{"kind":"noise","stops":[
                     {"pos":0,"color":{"model":"rgb","comps":[0,0,0,0]}},
                     {"pos":1,"color":{"model":"rgb","comps":[1,1,1,0]}}]}}"#,
                "noise",
            ),
        ] {
            let e = doc_with(arm);
            let err = appname_core::io::png_io::export_png_bytes(&e)
                .expect_err("this arm has nothing to composite FROM");
            assert_eq!(
                err.code,
                appname_core::engine::error::ErrorCode::NotImplemented,
                "arm {name} must refuse TYPED (QA-F8), got {err:?}"
            );
            assert!(
                err.message.contains(needle),
                "arm {name}'s refusal must name its OWN cause {needle:?}: {}",
                err.message
            );
        }

        // -- and the served solid arm really DRAWS ----------------------
        //
        // Without this the whole test would still pass if `export_png_bytes`
        // succeeded while omitting the layer — the QA-F8 silent-omit defect
        // this lane already shipped once. Pure red can only be the fill:
        // the background is white.
        let e = doc_with(r#"{"type":"solid","color":{"model":"rgb","comps":[1,0,0,0]}}"#);
        let png = appname_core::io::png_io::export_png_bytes(&e).expect("solid serves");
        let (buf, w, h) = appname_core::decode_png_to_buffer(&png, appname_core::ProfileId::nil())
            .expect("decode");
        assert_eq!((w, h), (16, 16));
        let px = buf.read_pixel_f32(appname_core::IVec2::new(8, 8));
        assert!(
            px[0] > 0.99 && px[1] < 0.01 && px[2] < 0.01 && px[3] > 0.99,
            "the exported PNG must actually BE the fill colour, got {px:?} \
             (white = the layer was silently omitted)"
        );
    }

    /// The constructor is the historical chain, byte-for-byte in effect:
    /// same kind set, same op set, vector service present. (Construction
    /// panics — duplicate kind/service registration — double as the
    /// structural guard; this pins the CONTENT.)
    #[test]
    fn production_assembly_matches_the_proven_chain() {
        let ours = production_assembly();
        // The historical spelling, verbatim (the four sites' old bodies).
        let historical = appname_vector::register(appname_adjust::register(
            appname_filters::register(appname_tools::kind_impls::with_tools_fill(
                appname_core::engine::kinds::register(appname_tools::kind_impls::assembly()),
            )),
        ));
        let mut a: Vec<&str> = ours.kinds().collect();
        let mut b: Vec<&str> = historical.kinds().collect();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b, "kind sets identical");
        let mut oa: Vec<&str> = ours.ops().ids().collect();
        let mut ob: Vec<&str> = historical.ops().ids().collect();
        oa.sort_unstable();
        ob.sort_unstable();
        assert_eq!(oa, ob, "op sets identical");
        assert!(
            ours.vector_geometry().is_some(),
            "vector service registered"
        );
        assert_eq!(
            ours.vector_geometry().map(|s| s.schema()),
            historical.vector_geometry().map(|s| s.schema()),
        );
        // Door C4 (slice 3 addendum): the kernel seam is injected and
        // carries EXACTLY render's exported fns (fn-pointer identity —
        // the one-math-library clause, pinned at the injection site).
        let routing = ours
            .blend_kernel_routing()
            .expect("§14.4 kernel routing registered on the production assembly");
        assert!(
            std::ptr::fn_addr_eq(
                routing.separable,
                appname_render::render_separable_kernel
                    as fn(appname_core::BlendMode) -> Option<appname_core::SeparableKernel>
            ),
            "separable routing IS render's export"
        );
        assert!(
            std::ptr::fn_addr_eq(
                routing.triple,
                appname_render::render_triple_kernel
                    as fn(appname_core::BlendMode) -> Option<appname_core::TripleKernel>
            ),
            "triple routing IS render's export"
        );
        // The TEXTVEC row (post-dates the historical five-wrap chain —
        // the manifest is the historical chain PLUS the text service).
        assert_eq!(
            ours.text_layout().map(|s| s.schema()),
            Some("appname.text.v1"),
            "TextLayout registered and version-stamped"
        );
        // Composite-leg slice: the fill-math seam is injected and carries
        // EXACTLY appname-text's exported fns (fn-pointer identity — the
        // one-math-library clause, pinned at the injection site; the C4
        // kernel-seam precedent above).
        let fc = ours
            .fill_convert_routing()
            .expect("fill-math routing registered on the production assembly");
        assert!(
            std::ptr::fn_addr_eq(
                fc.fill_to_linear_rgba,
                appname_text::fill_to_linear_rgba as appname_core::FillToLinearFn
            ),
            "fill_to_linear_rgba routing IS appname-text's export"
        );
        assert!(
            std::ptr::fn_addr_eq(
                fc.premultiply_coverage,
                appname_text::premultiply_coverage as appname_core::PremultiplyCoverageFn
            ),
            "premultiply_coverage routing IS appname-text's export"
        );
    }

    /// THE POSITIVE HALF of the parity guard: every shipped surface
    /// calls a production constructor — either `production_assembly()` or
    /// the handle variant `production_assembly_with_text_handle()` (both
    /// fold the SAME PRODUCTION_CHAIN, so either satisfies the "no
    /// hand-rolled chain" invariant; the negative-half scan below is what
    /// actually forbids drift). doc.rs (cabi) may route through the
    /// handle variant for the request-10 font door.
    #[test]
    fn every_shipped_surface_calls_the_one_constructor() {
        let root = workspace_root();
        let call_sites = [
            "crates/appname-cli/src/main.rs",
            "crates/appname-cabi/src/doc.rs",
            "crates/appname-script/src/bin/actions.rs",
            "tests/conformance/src/lib.rs",
            "crates/appname-script/tests/actions.rs",
        ];
        for rel in call_sites {
            let src = std::fs::read_to_string(root.join(rel))
                .unwrap_or_else(|e| panic!("call-site {rel} must be readable: {e}"));
            assert!(
                src.contains("appname_assembly::production_assembly()")
                    || src.contains("appname_assembly::production_assembly_with_text_handle("),
                "{rel} must call A production constructor (production_assembly or \
                 production_assembly_with_text_handle)"
            );
        }
    }

    /// CHAIN-EQUALITY PIN (the hard requirement): the two constructors
    /// produce the SAME EngineAssembly composition — the handle variant
    /// is production_assembly() plus the retained handle, never a
    /// divergent fold. If the manifest ever drifted between them (e.g. a
    /// row reachable by only one), this fails.
    #[test]
    fn the_two_constructors_compose_identically() {
        let plain = production_assembly();
        let (with_handle, _svc) = production_assembly_with_text_handle();
        let mut a: Vec<&str> = plain.kinds().collect();
        let mut b: Vec<&str> = with_handle.kinds().collect();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b, "kind sets identical across constructors");
        let mut oa: Vec<&str> = plain.ops().ids().collect();
        let mut ob: Vec<&str> = with_handle.ops().ids().collect();
        oa.sort_unstable();
        ob.sort_unstable();
        assert_eq!(oa, ob, "op sets identical across constructors");
        assert_eq!(
            plain.vector_geometry().map(|s| s.schema()),
            with_handle.vector_geometry().map(|s| s.schema()),
            "vector service identical",
        );
        assert_eq!(
            plain.text_layout().map(|s| s.schema()),
            with_handle.text_layout().map(|s| s.schema()),
            "text service identical (both register the version-stamped seam)",
        );
        assert_eq!(
            plain.blend_kernel_routing().is_some(),
            with_handle.blend_kernel_routing().is_some(),
            "kernel seam identical",
        );
        assert_eq!(
            plain.fill_convert_routing().is_some(),
            with_handle.fill_convert_routing().is_some(),
            "fill seam identical",
        );
    }

    /// THE NEGATIVE HALF (review MAJOR-1 ruled fix): a WORKSPACE-WALK
    /// scan — no hand-rolled chain spelling ANYWHERE outside this crate
    /// and the allowlist below. The first cut iterated a closed
    /// five-file array; this walk is what makes the module doc's
    /// "anywhere turns a test red" claim true, and dry-running it found
    /// a FOURTH stale copy the review's three had not named
    /// (appname-adjust's auto_kinds rig — swapped with the ruled three).
    #[test]
    fn no_hand_rolled_chain_anywhere_in_the_workspace() {
        // The fingerprint: the qualified wrap-call spellings every
        // hand-rolled chain in tree history has used. `register_ops(`
        // etc. do not match (the open paren binds the token).
        const FINGERPRINT: [&str; 5] = [
            "appname_vector::register(",
            "appname_filters::register(",
            "appname_adjust::register(",
            // Door C4: a hand-rolled kernel injection is the same drift
            // class — the production wrap lives ONLY in this crate's
            // manifest row.
            ".with_blend_kernels(",
            "appname_text::register(",
        ];
        // Deliberate-partial rigs, ALLOWLISTED BY EXACT PATH — each rig
        // site carries a comment naming its entry here. Everything else
        // that wants production wraps either calls
        // `production_assembly()` or folds `PRODUCTION_CHAIN` (no
        // fingerprint tokens either way).
        const ALLOW: [&str; 2] = [
            // Renderer-side probe: filters+adjust ops over BARE core
            // (deliberately no tools/vector — §12.1 op-eval fixtures on
            // doc.new grounds; never claims production parity).
            "crates/appname-render/tests/parametric_composite.rs",
            // Core's apply-image battery: deliberate-partial rig — core
            // cannot fold PRODUCTION_CHAIN (dependency direction) and
            // the cyclic dev-dep type split forces serde-bridge
            // adapters; comment at the rig site names this entry.
            "crates/appname-core/src/engine/kinds_apply_image.rs",
        ];
        let root = workspace_root();
        let mut offenders = Vec::new();
        for dir in ["crates", "tests"] {
            walk_rs(&root.join(dir), &mut |path| {
                let rel = path
                    .strip_prefix(&root)
                    .expect("walked under root")
                    .to_string_lossy()
                    .replace('\\', "/");
                if rel.starts_with("crates/appname-assembly/") || ALLOW.contains(&rel.as_str()) {
                    return;
                }
                let src = std::fs::read_to_string(path).unwrap_or_default();
                if FINGERPRINT.iter().any(|t| src.contains(t)) {
                    offenders.push(rel);
                }
            });
        }
        offenders.sort();
        assert!(
            offenders.is_empty(),
            "hand-rolled production-chain spelling(s) outside appname-assembly \
             (the MAJOR-1 drift class): {offenders:?} — call \
             production_assembly(), fold PRODUCTION_CHAIN for a synced \
             partial rig, or allowlist a deliberate-partial rig HERE with a \
             comment at the rig site"
        );
    }

    /// PRODUCTION-SERVED end-to-end (composite-leg slice, the wall falls):
    /// on the REAL production assembly — real VectorGeometry + the real
    /// appname-text fill math, no probes — a shape layer created through
    /// `vector.shape_add` COMPOSITES instead of walling. This is the
    /// definitive flip: before the fill-seam row, `export_png_bytes` on a
    /// shape-bearing document refused typed (the QA-F8 wall); now it
    /// serves. Byte-level correctness of the coverage×fill product is
    /// pinned in core's probe test; this proves the PRODUCTION wiring
    /// reaches the serving arm.
    #[test]
    fn production_assembly_serves_shape_composites_the_wall_falls() {
        let mut e = production_assembly().build();
        e.apply_json(
            r#"{"schema":"appname.command.v1","id":"a55e0000-0000-4000-8000-000000000001","kind":"doc.new","params":{"width":32,"height":32,"background":"white","depth":"f32"}}"#,
        )
        .expect("doc.new");
        // A real ellipse shape with a solid fill and the contract's
        // default zero-width stroke (validMinimal shape) — through the
        // live §20 geometry service.
        e.apply_json(
            r#"{"schema":"appname.command.v1","id":"a55e0000-0000-4000-8000-000000000002","kind":"vector.shape_add","params":{"primitive":{"kind":"ellipse","geometry":{"x":4,"y":4,"w":16,"h":16}},"fill":{"type":"solid","color":{"model":"rgba","comps":[0,0.5,1,1]}},"stroke":{"widthPx":0},"newLayerId":"a55e0000-0000-4000-8000-0000000000f1"}}"#,
        )
        .expect("vector.shape_add on the production assembly");
        // The wall is GONE: the composite serves (was a typed refusal
        // before the fill-seam row). Non-empty PNG bytes = a served
        // composite; the shape's fill lands somewhere inside the ellipse.
        let png = appname_core::export_png_bytes(&e)
            .expect("production shape composite SERVES (the wall fell)");
        assert!(png.len() > 8, "served composite emits real PNG bytes");
    }

    fn workspace_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root")
            .to_path_buf()
    }

    /// Depth-first `.rs` walk, skipping build output (`target/`).
    fn walk_rs(dir: &std::path::Path, f: &mut impl FnMut(&std::path::Path)) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                if p.file_name().and_then(|n| n.to_str()) != Some("target") {
                    walk_rs(&p, f);
                }
            } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
                f(&p);
            }
        }
    }
}
