// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Q-1 BOUNDS-DOOR CENSUS GUARD (the strong-form census bar): the class
// behind CRITICAL Q-1 / aud-replay F1 — writes-through-soft-coverage
// forking `state_hash` across save→reopen — was reintroduced twice by NEW
// call sites of the bounds-maintenance doors that were each individually
// "reasonable" (an eager grow by a pre-quantization rect; a tile-granular
// shrink after an alpha-reducing write). Reviews did not catch them; probes
// without the feathered-selection arm did not catch them.
//
// This test makes the census MECHANICAL: it scans every `crates/*/src`
// Rust source for call sites of the bounds-mutating doors and compares the
// per-file counts against the audited manifest below. A NEW call site (or a
// moved/removed one) REDS this test until the manifest is re-audited —
// which is the point: the audit question for any new site is
//
//   "does the extent this site leaves behind equal the package loader's
//    `recompute_bounds_pixel_exact` over the stored (quantized) plane,
//    for every selection state (none / feathered / rect) and every
//    content state the op can reach?"
//
// If the answer is not an easy yes, route the site through
// `appname_tools::kind_impls::finish_write_bounds` (raster write commits)
// or the `KindContext` pixel-exact primitives, add a probe to
// `replay_stroke_kinds_pin.rs`, and only then update the manifest.
//
// Audited manifest legend (why each existing site is safe):
//   [pixel-exact]   the pixel-exact primitives themselves — loader-agreeing
//                   by definition.
//   [paired]        eager grow immediately followed by a pixel-exact
//                   re-tighten on the same path (F1 fill, W12 bake, W18
//                   paste).
//   [opaque-write]  writes exact alpha=1 pixels only — every counted pixel
//                   stores nonzero, the extent cannot run proud
//                   (doc.canvas_resize; probed with the feathered arm).
//   [pre-tighten]   eager per-tile grow with a single pixel-exact re-tighten
//                   after the loop (content_aware_fill).
//   [selection]     selection/channel MASK surfaces (§13.1): their bounds
//                   are maintained by the §13.1 discipline and re-derived
//                   via sync_selection_bounds; not part of the raster-layer
//                   region_hash fork class. (If a future finding shows a
//                   persisted-selection fork, these get their own probes.)
//   [test-mock]     inside #[cfg(test)] scaffolding.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The door names whose CALL SITES this census pins. Method-call syntax
/// (leading dot) so definitions do not count; rustfmt never separates a
/// method name from its opening paren.
const DOORS: &[&str] = &[
    ".grow_layer_bounds(",
    ".grow_surface_bounds(",
    ".recompute_layer_bounds(",
    ".recompute_surface_bounds(",
    ".set_surface_bounds(",
    ".grow_layer_bounds_pixel_exact(",
    ".recompute_layer_bounds_pixel_exact(",
];

/// The audited manifest: (file, door, count). Sorted by (file, door).
const AUDITED: &[(&str, &str, usize)] = &[
    // KindContext: the wrappers' own delegation to the surface-generic
    // doors (definitions, not kind call sites) + §13.1 selection ops.
    // [pixel-exact]/[selection]
    (
        "appname-core/src/engine/kind_impl.rs",
        ".grow_surface_bounds(",
        3,
    ),
    (
        "appname-core/src/engine/kind_impl.rs",
        ".recompute_surface_bounds(",
        3,
    ),
    (
        "appname-core/src/engine/kind_impl.rs",
        ".set_surface_bounds(",
        4,
    ),
    // Core kinds: FillKind grow [paired: F1], edit.paste grow [paired: W18].
    ("appname-core/src/engine/kinds.rs", ".grow_layer_bounds(", 2),
    // FillKind [paired], edit.paste [paired], edit.cut (Q-1 shrink fix),
    // mask.apply (Q-1 shrink fix). [pixel-exact]
    (
        "appname-core/src/engine/kinds.rs",
        ".recompute_layer_bounds_pixel_exact(",
        4,
    ),
    // §13.1 selection-surface exact sets. [selection]
    (
        "appname-core/src/engine/kinds.rs",
        ".set_surface_bounds(",
        2,
    ),
    // apply_image channel writes: exact in-pass tight rects. [selection]
    (
        "appname-core/src/engine/kinds_apply_image.rs",
        ".set_surface_bounds(",
        2,
    ),
    // §14 channel selection ops. [selection]
    (
        "appname-core/src/engine/kinds_channels.rs",
        ".grow_surface_bounds(",
        1,
    ),
    (
        "appname-core/src/engine/kinds_channels.rs",
        ".recompute_surface_bounds(",
        1,
    ),
    (
        "appname-core/src/engine/kinds_channels.rs",
        ".set_surface_bounds(",
        2,
    ),
    // §19 layer-geometry exact set. [pixel-exact-equivalent: exact rect]
    (
        "appname-core/src/engine/kinds_layer_geom.rs",
        ".set_surface_bounds(",
        1,
    ),
    // Merge family: receiver scoped grow + bake_own_mask shrink (Q-1 fix).
    // [pixel-exact]
    (
        "appname-core/src/engine/kinds_merge.rs",
        ".grow_layer_bounds_pixel_exact(",
        1,
    ),
    (
        "appname-core/src/engine/kinds_merge.rs",
        ".recompute_layer_bounds_pixel_exact(",
        1,
    ),
    // In-crate mock kind inside `mod tests`. [test-mock]
    ("appname-core/src/engine/mod.rs", ".grow_layer_bounds(", 1),
    // Tools: install_plane bake [paired: W12], fill [paired: F1 +
    // needs_pixel_exact guard], doc.canvas_resize [opaque-write],
    // content_aware_fill per-tile eager [pre-tighten].
    ("appname-tools/src/kind_impls.rs", ".grow_layer_bounds(", 4),
    // finish_write_bounds' grow branch. [pixel-exact]
    (
        "appname-tools/src/kind_impls.rs",
        ".grow_layer_bounds_pixel_exact(",
        1,
    ),
    // Selection-surface op. [selection]
    (
        "appname-tools/src/kind_impls.rs",
        ".grow_surface_bounds(",
        1,
    ),
    // install_plane [paired: W12], fill [paired: F1], finish_write_bounds'
    // erase branch. [pixel-exact]
    (
        "appname-tools/src/kind_impls.rs",
        ".recompute_layer_bounds_pixel_exact(",
        3,
    ),
    // Selection-surface prune. [selection]
    (
        "appname-tools/src/kind_impls.rs",
        ".recompute_surface_bounds(",
        1,
    ),
    // Exact selection-surface sets. [selection]
    ("appname-tools/src/kind_impls.rs", ".set_surface_bounds(", 2),
];

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = <root>/crates/appname-cli
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("readable source tree") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn bounds_door_call_sites_match_the_audited_manifest() {
    let root = workspace_root();
    let crates_dir = root.join("crates");
    assert!(
        crates_dir.is_dir(),
        "census scanner lost the workspace root at {crates_dir:?}"
    );

    let mut files = Vec::new();
    for entry in std::fs::read_dir(&crates_dir).expect("crates dir") {
        let src = entry.expect("crate dir").path().join("src");
        if src.is_dir() {
            rust_sources(&src, &mut files);
        }
    }
    files.sort();

    let mut found: BTreeMap<(String, &'static str), usize> = BTreeMap::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("readable source");
        let rel = file
            .strip_prefix(&crates_dir)
            .expect("under crates/")
            .to_string_lossy()
            .replace('\\', "/");
        for door in DOORS {
            let n = text.match_indices(door).count();
            if n > 0 {
                *found.entry((rel.clone(), door)).or_insert(0) += n;
            }
        }
    }

    let mut audited: BTreeMap<(String, &'static str), usize> = BTreeMap::new();
    for (file, door, n) in AUDITED {
        audited.insert(((*file).to_string(), door), *n);
    }

    if found != audited {
        let mut report = String::new();
        for (k, n) in &found {
            match audited.get(k) {
                Some(a) if a == n => {}
                Some(a) => report.push_str(&format!(
                    "  CHANGED  {} {} — audited {a}, found {n}\n",
                    k.0, k.1
                )),
                None => report.push_str(&format!("  NEW      {} {} — found {n}\n", k.0, k.1)),
            }
        }
        for (k, a) in &audited {
            if !found.contains_key(k) {
                report.push_str(&format!("  REMOVED  {} {} — audited {a}\n", k.0, k.1));
            }
        }
        panic!(
            "bounds-door call sites drifted from the audited census (Q-1 class guard).\n\
             \n{report}\n\
             A NEW or CHANGED site is a potential fifth-door reintroduction of the\n\
             save→reopen state_hash fork. Before updating the AUDITED manifest in\n\
             crates/appname-cli/tests/bounds_door_census.rs, verify the site leaves\n\
             bounds equal to the loader's pixel-exact recompute for every selection\n\
             state (none/feathered/rect) — route raster write commits through\n\
             appname_tools::kind_impls::finish_write_bounds or the KindContext\n\
             pixel-exact doors, and add a probe to replay_stroke_kinds_pin.rs."
        );
    }
}
