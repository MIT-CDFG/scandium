// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Exemplar-engine test suite: mechanical rights-invariant enforcement
//! (source hygiene), behavioral invariant evidence (determinism,
//! order-invariance, fresh-candidate instrumentation), quality oracles
//! (constant / periodic / structure propagation / seam compositing /
//! decontamination / fallback), §45.3 seed-policy conformance (§16.4
//! rule 5), and pinned goldens (pinned AFTER the oracles were green).

use super::synth::{trace, Knobs};
use super::*;
use crate::plane::{plane_hash, Plane, SplitMix64};

// ---------------------------------------------------------------- fixtures

fn rect_mask(w: usize, h: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> Plane {
    let mut m = Plane::new(w, h);
    for y in y0..y1 {
        for x in x0..x1 {
            m.data[y * w + x] = 1.0;
        }
    }
    m
}

fn disk_mask(w: usize, h: usize, cx: f64, cy: f64, r: f64) -> Plane {
    let mut m = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt();
            m.data[y * w + x] = if d <= r { 1.0 } else { 0.0 };
        }
    }
    m
}

/// Vertical stripes, period `2 * half` (values 0.25 / 0.75).
fn stripes(w: usize, h: usize, half: usize) -> Plane {
    let mut p = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            p.data[y * w + x] = if (x / half).is_multiple_of(2) {
                0.25
            } else {
                0.75
            };
        }
    }
    p
}

fn noise(w: usize, h: usize, seed: u64) -> Plane {
    let mut rng = SplitMix64(seed);
    let mut p = Plane::new(w, h);
    for v in p.data.iter_mut() {
        *v = (0.2 + 0.6 * rng.next_f64()) as f32;
    }
    p
}

/// Overwrite the hole with a constant "object" the fill must remove.
fn corrupt(p: &mut Plane, hole: &Plane, value: f32) {
    for (v, &m) in p.data.iter_mut().zip(hole.data.iter()) {
        if m >= 0.5 {
            *v = value;
        }
    }
}

/// Mean |out − truth| over hole pixels with BFS depth ≥ 3 (outside the
/// §45.3 2-px feather band, where the ramp mixes in original content by
/// design).
fn interior_err(out: &Plane, truth: &Plane, hole: &Plane) -> f64 {
    let hb: Vec<bool> = hole.data.iter().map(|&v| v >= 0.5).collect();
    let depth = super::diffuse::hole_depth(&hb, hole.w, hole.h);
    let mut acc = 0f64;
    let mut n = 0usize;
    for i in 0..hole.w * hole.h {
        if hb[i] && depth[i] != u32::MAX && depth[i] >= 3 {
            acc += (out.data[i] as f64 - truth.data[i] as f64).abs();
            n += 1;
        }
    }
    assert!(n > 0, "fixture has no interior pixels");
    acc / n as f64
}

fn no_adapt() -> ExemplarParams {
    ExemplarParams {
        color_adaptation: ColorAdaptation::None,
        ..ExemplarParams::default()
    }
}

// ------------------------------------------- mechanical invariant evidence

/// Crate-side "grep leg" of the contract's invariant enforcement: the
/// module's own sources must never carry the walled identifier classes
/// (the randomized neighbor-field engine name, the bare acronym for a
/// neighbor field, the carving stem) nor any RNG crate usage. Patterns
/// are assembled from fragments so this test file itself stays clean.
#[test]
fn mechanical_source_hygiene() {
    let sources: [(&str, &str); 10] = [
        ("mod.rs", include_str!("mod.rs")),
        ("stream.rs", include_str!("stream.rs")),
        ("pyramid.rs", include_str!("pyramid.rs")),
        ("diffuse.rs", include_str!("diffuse.rs")),
        ("descriptor.rs", include_str!("descriptor.rs")),
        ("kdtree.rs", include_str!("kdtree.rs")),
        ("synth.rs", include_str!("synth.rs")),
        ("seam.rs", include_str!("seam.rs")),
        ("adapt.rs", include_str!("adapt.rs")),
        ("tests.rs", include_str!("tests.rs")),
    ];
    // Walled stems (plain substring on lowercased text — stricter than
    // the gate's delimiter-bounded sweep for these tokens, none of which
    // occur as an innocent substring in this module).
    let walled: [String; 3] = [
        ["patch", "match"].concat(),
        ["n", "n", "f"].concat(),
        ["seam", "carv"].concat(),
    ];
    // RNG crate usage in any form.
    let rng: [String; 3] = [
        ["use ", "ra", "nd"].concat(),
        ["ra", "nd", "::"].concat(),
        ["extern crate ", "ra", "nd"].concat(),
    ];
    for (name, text) in sources {
        let lower = text.to_lowercase();
        for pat in walled.iter().chain(rng.iter()) {
            assert!(
                !lower.contains(pat.as_str()),
                "walled/RNG token class found in {name}"
            );
        }
    }
    // The crate manifest must not grow an RNG dependency either.
    let manifest = include_str!("../../Cargo.toml").to_lowercase();
    assert!(!manifest.contains(["ra", "nd"].concat().as_str()));
}

// ------------------------------------------- behavioral invariant evidence

/// (a) Two-run byte-identity on every fixture family (D-48).
#[test]
fn determinism_two_run_byte_identity() {
    let hole = rect_mask(44, 44, 17, 17, 27, 27);
    let mut img = stripes(44, 44, 4);
    corrupt(&mut img, &hole, 0.5);
    let planes = vec![img];
    for params in [
        ExemplarParams::default(),
        no_adapt(),
        ExemplarParams {
            quality: Quality::Best,
            ..ExemplarParams::default()
        },
        ExemplarParams {
            quality: Quality::Draft,
            ..ExemplarParams::default()
        },
        ExemplarParams {
            mirror: true,
            ..ExemplarParams::default()
        },
        ExemplarParams {
            rotation: RotationAdaptation::Low,
            ..ExemplarParams::default()
        },
    ] {
        let a = exemplar_fill(&planes, None, &hole, None, &params).unwrap();
        let b = exemplar_fill(&planes, None, &hole, None, &params).unwrap();
        assert_eq!(
            plane_hash(&a.planes[0].data),
            plane_hash(&b.planes[0].data),
            "two-run identity broke for {params:?}"
        );
    }
}

/// (b) Permuting the engine's internal candidate-evaluation order leaves
/// the output byte-identical: the winner is a total-order argmin over a
/// per-query set, so no hidden order-dependent state can exist between
/// iterations (§16.4 rule 5 "permuted evaluation order ⇒ identical").
#[test]
fn candidate_order_permutation_is_invisible() {
    let hole = rect_mask(44, 44, 17, 17, 27, 27);
    let mut img = stripes(44, 44, 4);
    corrupt(&mut img, &hole, 0.5);
    let planes = vec![img];
    let params = ExemplarParams::default();
    let (a, _) = fill_with_knobs(&planes, None, &hole, None, &params, Knobs::default()).unwrap();
    let (b, _) = fill_with_knobs(
        &planes,
        None,
        &hole,
        None,
        &params,
        Knobs {
            reverse_candidates: true,
            ..Knobs::default()
        },
    )
    .unwrap();
    assert_eq!(
        plane_hash(&a.planes[0].data),
        plane_hash(&b.planes[0].data),
        "candidate scan order leaked into the output"
    );
}

/// (c) Instrumentation: every query rebuilds its candidate set fresh —
/// query count equals candidate-set-build count exactly (nothing is ever
/// served from a per-pixel store because none exists), and a second
/// identical fill doubles both counters exactly (no cross-run or
/// cross-iteration carry-over).
#[test]
fn every_query_recomputes_candidates_fresh() {
    let hole = rect_mask(36, 36, 14, 14, 22, 22);
    let mut img = stripes(36, 36, 4);
    corrupt(&mut img, &hole, 0.5);
    let planes = vec![img];
    let params = ExemplarParams::default();
    trace::reset();
    exemplar_fill(&planes, None, &hole, None, &params).unwrap();
    let (q1, b1) = trace::snapshot();
    assert!(q1 > 0, "no queries recorded");
    assert_eq!(q1, b1, "a query was answered without building its set");
    exemplar_fill(&planes, None, &hole, None, &params).unwrap();
    let (q2, b2) = trace::snapshot();
    assert_eq!(q2, 2 * q1, "second run did not recompute every query");
    assert_eq!(b2, 2 * b1);
}

/// (c′) Allocation discipline on the refine path ([PERF §45.3] fix
/// round, invariant-review defect 1): `refine` must NOT clone full-
/// canvas state per re-searched patch. The pre-fix implementation
/// constructed a fresh `Fill` per re-search, cloning four full-canvas
/// buffers (filled 1 B/px + conf 4 B/px + src 6 B/px + owner 4 B/px =
/// 15 B/px) per patch, per pass. The per-thread counting allocator
/// ([`crate::alloc_probe`]) bounds the WHOLE refine call's allocation
/// traffic below HALF of what a single pre-fix per-patch clone set
/// cost — so even one surviving canvas clone would fail this test.
/// (The remaining traffic is small per-patch bookkeeping: residual /
/// order vectors sized by patch count and k-d traversal nodes.)
#[test]
fn refine_does_not_clone_canvas_state() {
    let (w, h) = (128, 128);
    let hole_p = rect_mask(w, h, 56, 56, 72, 72);
    let mut img = stripes(w, h, 4);
    corrupt(&mut img, &hole_p, 0.5);
    let mut planes = vec![img];
    let hole_b: Vec<bool> = hole_p.data.iter().map(|&v| v >= 0.5).collect();
    let guard = dilate(&hole_b, w, h, 2);
    let s: Vec<bool> = guard.iter().map(|&g| !g).collect();
    // Mirror fill_impl's per-level stage: diffuse init, libraries,
    // priority-front synthesis, then the R = 2 (Best-preset) refine.
    super::diffuse::bfs_diffuse(&mut planes, &hole_b, w, h);
    let key = super::stream::level_key(0, 0);
    let libs = super::synth::build_libraries(&planes, &s, w, h, &[], &[], false, &key);
    let mut synth = super::synth::synthesize_level(
        &mut planes,
        &hole_b,
        w,
        h,
        &libs,
        0.75,
        8,
        &key,
        Knobs::default(),
    );
    assert!(
        !synth.patches.is_empty(),
        "fixture committed no patches; refine would be vacuous"
    );
    let before = crate::alloc_probe::allocated_on_this_thread();
    super::synth::refine(
        &mut planes,
        &hole_b,
        w,
        h,
        &libs,
        0.75,
        8,
        Knobs::default(),
        &mut synth,
        2,
    );
    let used = crate::alloc_probe::allocated_on_this_thread() - before;
    let one_clone_set = (15 * w * h) as u64;
    eprintln!(
        "refine alloc traffic: {used} B over 2 passes ({} patches); one pre-fix per-patch clone set = {one_clone_set} B",
        synth.patches.len()
    );
    assert!(
        used < one_clone_set / 2,
        "refine allocated {used} B; a SINGLE pre-fix per-patch canvas-clone set was {one_clone_set} B"
    );
}

/// §45.3 transform-cap drop priority (spec conformance, fix-round defect
/// 4; ORCHESTRATOR RULING: conform): "beyond the cap the coarsest-scale
/// libraries drop first." Oracle: size every candidate scale library
/// independently, replicate the normative drop rule (coarsest = smallest
/// scale factor first, until the kept set fits the 2×-base budget), and
/// require the engine's kept/dropped sets to match exactly. The pre-fix
/// enumeration-order skip would keep the COARSE prefix instead — this
/// fixture distinguishes the two.
#[test]
fn transform_cap_drops_coarsest_scale_first() {
    use super::synth::{build_libraries, transform_validity, InvMap};
    let (w, h) = (48, 48);
    let planes = vec![noise(w, h, 0xCA9)];
    let s_valid = vec![true; w * h];
    // §45.3 scale set, ascending (also the enumeration order).
    let scales = [0.5f64, 2f64.powf(-0.5), 2f64.powf(0.5), 2.0];
    let counts: Vec<usize> = scales
        .iter()
        .map(|&s| {
            let map = InvMap::new(w, h, 0.0, s, false);
            super::descriptor::valid_centers(&transform_validity(&s_valid, w, h, &map), w, h).len()
        })
        .collect();
    let base = super::descriptor::valid_centers(&s_valid, w, h).len();
    let budget = 2 * base;
    let total: usize = counts.iter().sum();
    assert!(
        total > budget,
        "fixture must exceed the cap (total {total} vs budget {budget})"
    );
    // Normative drop order: coarsest scale first, until it fits.
    let mut keep = [true; 4];
    let mut expect_dropped = 0usize;
    let mut t = total;
    for i in 0..4 {
        if t <= budget {
            break;
        }
        keep[i] = false;
        expect_dropped += 1;
        t -= counts[i];
    }
    assert!(t <= budget, "fixture cannot fit even after drops");
    assert!(expect_dropped > 0);
    let key = super::stream::level_key(0, 0);
    let libs = build_libraries(&planes, &s_valid, w, h, &[], &scales, false, &key);
    assert_eq!(
        libs.dropped, expect_dropped,
        "dropped-library count must match the coarsest-first rule"
    );
    // Kept set = identity + the FINEST scales, in enumeration order,
    // identified by their center counts (distinct per scale here).
    let expect_counts: Vec<usize> = std::iter::once(base)
        .chain(
            counts
                .iter()
                .zip(keep.iter())
                .filter(|&(_, &k)| k)
                .map(|(&c, _)| c),
        )
        .collect();
    let got_counts: Vec<usize> = libs.libs.iter().map(|l| l.centers.len()).collect();
    assert_eq!(
        got_counts, expect_counts,
        "kept libraries must be identity + the finest scales (coarsest dropped first)"
    );
}

/// Tie-rich fixture with genuinely ambiguous completions: two
/// complementary channels of Bernoulli vertical stripes whose LUMA is
/// exactly constant — the isophote term is the ε floor at every front
/// pixel, so priorities are purely geometric and bit-tied in droves; pop
/// order is then decided by the §45.3 seeded draw, and because stripe
/// contexts repeat with different continuations, different pop orders
/// commit different (equally plausible) content. The hole is a
/// full-height band so no vertical context pins the completion.
fn tie_band_fixture() -> (Vec<Plane>, Plane) {
    let (w, h) = (44, 44);
    let mut rng = SplitMix64(0x5EED);
    let levels: Vec<f32> = (0..11)
        .map(|_| if rng.next_u64() & 1 == 0 { 0.3 } else { 0.7 })
        .collect();
    let mut ch0 = Plane::new(w, h);
    let mut ch1 = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            ch0.data[y * w + x] = levels[x / 4];
            ch1.data[y * w + x] = 1.0 - levels[x / 4];
        }
    }
    let hole = rect_mask(w, h, 16, 0, 28, h);
    corrupt(&mut ch0, &hole, 0.5);
    corrupt(&mut ch1, &hole, 0.5);
    (vec![ch0, ch1], hole)
}

/// (d) §45.3 seed policy / §16.4 rule 5: same seed bit-identical
/// (covered above), different seed measurably different where ties exist
/// (see [`tie_band_fixture`]).
#[test]
fn different_seed_differs_where_ties_exist() {
    let (planes, hole) = tie_band_fixture();
    let a = exemplar_fill(
        &planes,
        None,
        &hole,
        None,
        &ExemplarParams {
            seed: 0,
            ..no_adapt()
        },
    )
    .unwrap();
    let b = exemplar_fill(
        &planes,
        None,
        &hole,
        None,
        &ExemplarParams {
            seed: 1,
            ..no_adapt()
        },
    )
    .unwrap();
    assert_ne!(
        plane_hash(&a.planes[0].data),
        plane_hash(&b.planes[0].data),
        "tie-rich fixture must respond to the seed"
    );
}

// ------------------------------------------------------------ quality oracles

/// Constant region fills exactly. Two claims: (1) with hole content
/// already equal to the surroundings, EVERY pixel is exact (the feather
/// blends equals); (2) with a corrupted hole ("object"), every pixel
/// outside the §45.3 2-px feather band is exact — the band itself ramps
/// original content back in by the spec's own coverage rule.
#[test]
fn constant_region_fills_exactly() {
    let (w, h) = (28, 28);
    let hole = disk_mask(w, h, 14.0, 14.0, 5.0);
    let planes: Vec<Plane> = (0..3)
        .map(|c| Plane::from_vec(w, h, vec![0.2 + 0.2 * c as f32; w * h]))
        .collect();
    let out = exemplar_fill(&planes, None, &hole, None, &ExemplarParams::default()).unwrap();
    assert_eq!(out.method, FillMethod::Exemplar);
    for (c, plane) in out.planes.iter().enumerate() {
        let want = 0.2 + 0.2 * c as f32;
        for (i, &v) in plane.data.iter().enumerate() {
            assert!(
                (v - want).abs() < 1e-5,
                "channel {c} pixel {i}: {v} != {want}"
            );
        }
    }
    let mut corrupted = planes.clone();
    for p in corrupted.iter_mut() {
        corrupt(p, &hole, 0.9);
    }
    let out = exemplar_fill(&corrupted, None, &hole, None, &ExemplarParams::default()).unwrap();
    let hb: Vec<bool> = hole.data.iter().map(|&v| v >= 0.5).collect();
    let depth = super::diffuse::hole_depth(&hb, w, h);
    for (c, plane) in out.planes.iter().enumerate() {
        let want = 0.2 + 0.2 * c as f32;
        for (i, &v) in plane.data.iter().enumerate() {
            if hb[i] && depth[i] < 3 {
                continue; // feather band: ramps original content by spec
            }
            assert!(
                (v - want).abs() < 1e-5,
                "channel {c} pixel {i}: {v} != {want}"
            );
        }
    }
}

/// Periodic texture continues through the hole (interior error under a
/// tight bound; the 2-px feather band is excluded — it blends original
/// content by design).
#[test]
fn periodic_texture_continues() {
    let (w, h) = (44, 44);
    let hole = rect_mask(w, h, 17, 17, 27, 27);
    let truth = stripes(w, h, 4);
    let mut img = truth.clone();
    corrupt(&mut img, &hole, 0.5);
    let out = exemplar_fill(&[img], None, &hole, None, &no_adapt()).unwrap();
    assert_eq!(out.method, FillMethod::Exemplar);
    let err = interior_err(&out.planes[0], &truth, &hole);
    assert!(err < 0.05, "stripe continuation error {err}");
}

/// A linear structure (dark bar) continues through the hole, and the
/// isophote-driven priority order beats plain onion peel on exactly this
/// fixture (the criminisi04 headline property, justifying the data
/// term): onion peel closes the hole from all sides with background
/// before the bar tips can bridge; the priority front pushes the bar
/// first. Mild deterministic texture breaks exact-match degeneracy so
/// the schedules can actually diverge. Ablation via the internal knob.
#[test]
fn edge_propagates_and_priority_beats_onion_peel() {
    let (w, h) = (48, 40);
    let mut rng = SplitMix64(0xED6E);
    let mut truth = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let base = if (18..22).contains(&y) { 0.15 } else { 0.85 };
            truth.data[y * w + x] = base + 0.04 * rng.next_f64() as f32;
        }
    }
    let hole = rect_mask(w, h, 16, 13, 32, 27);
    let mut img = truth.clone();
    corrupt(&mut img, &hole, 0.5);
    let planes = vec![img];
    let params = no_adapt();
    let (with_priority, _) =
        fill_with_knobs(&planes, None, &hole, None, &params, Knobs::default()).unwrap();
    let (onion, _) = fill_with_knobs(
        &planes,
        None,
        &hole,
        None,
        &params,
        Knobs {
            onion_peel: true,
            ..Knobs::default()
        },
    )
    .unwrap();
    let err_p = interior_err(&with_priority.planes[0], &truth, &hole);
    let err_o = interior_err(&onion.planes[0], &truth, &hole);
    assert_ne!(
        plane_hash(&with_priority.planes[0].data),
        plane_hash(&onion.planes[0].data),
        "priority schedule and onion peel must actually differ here"
    );
    assert!(err_p < 0.15, "bar continuation error {err_p}");
    assert!(
        err_p <= err_o,
        "priority ({err_p}) must not lose to onion peel ({err_o})"
    );
}

/// The Kwatra boundary energy over ownership seams (the objective the
/// §45.3 seam pass minimizes): Σ over 4-connected pairs with at least
/// one endpoint in the hole and differing owners of
/// `‖A(p)−B(p)‖ + ‖A(q)−B(q)‖`, where a label's color is its committed
/// source pixel (identity-library sources live outside the hole, so the
/// INPUT image provides them) and the buffer value elsewhere.
fn modeled_seam_energy(
    out: &Plane,
    input: &Plane,
    hole: &[bool],
    w: usize,
    h: usize,
    owner: &[u32],
    patches: &[super::synth::PatchRec],
) -> f64 {
    use super::synth::NO_OWNER;
    let r = super::descriptor::PATCH_RADIUS as isize;
    let lc = |o: u32, x: usize, y: usize| -> f64 {
        if o != NO_OWNER {
            let pr = &patches[o as usize];
            let dx = x as isize - pr.tx as isize;
            let dy = y as isize - pr.ty as isize;
            if dx.abs() <= r && dy.abs() <= r {
                let lx = (pr.sx as isize + dx) as usize;
                let ly = (pr.sy as isize + dy) as usize;
                return input.data[ly * w + lx] as f64;
            }
        }
        out.data[y * w + x] as f64
    };
    let mut acc = 0f64;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            for (dx, dy) in [(1isize, 0isize), (0, 1)] {
                let (xx, yy) = (x as isize + dx, y as isize + dy);
                if xx >= w as isize || yy >= h as isize {
                    continue;
                }
                let j = yy as usize * w + xx as usize;
                if !hole[i] && !hole[j] {
                    continue;
                }
                let (oa, ob) = (owner[i], owner[j]);
                if oa == ob {
                    continue;
                }
                let (qx, qy) = (xx as usize, yy as usize);
                acc +=
                    (lc(oa, x, y) - lc(ob, x, y)).abs() + (lc(oa, qx, qy) - lc(ob, qx, qy)).abs();
            }
        }
    }
    acc
}

/// Boundary-cut compositing beats naive paste: the graph-cut seam pass
/// strictly reduces the modeled Kwatra boundary energy on a
/// stochastic-texture fixture. Draft preset isolates the seam pass (no
/// refine, no adaptation). Ablation via the internal knob.
#[test]
fn seam_pass_reduces_visible_boundaries() {
    // Two-texture fixture: smooth vertical gradient left, stochastic
    // texture right; the hole straddles the junction, so the two
    // continuations collide inside it and the collision line is
    // relabel-able.
    let (w, h) = (48, 40);
    let mut img = noise(w, h, 0xCAF);
    for y in 0..h {
        for x in 0..24 {
            img.data[y * w + x] = 0.2 + 0.6 * y as f32 / h as f32;
        }
    }
    let hole = rect_mask(w, h, 16, 14, 32, 26);
    corrupt(&mut img, &hole, 0.5);
    let input = img.clone();
    let planes = vec![img];
    let params = ExemplarParams {
        quality: Quality::Draft,
        ..ExemplarParams::default()
    };
    let (with_seam, dbg_s) =
        fill_with_knobs(&planes, None, &hole, None, &params, Knobs::default()).unwrap();
    let (naive, dbg_n) = fill_with_knobs(
        &planes,
        None,
        &hole,
        None,
        &params,
        Knobs {
            no_seam: true,
            ..Knobs::default()
        },
    )
    .unwrap();
    let m_seam = modeled_seam_energy(
        &with_seam.planes[0],
        &input,
        &dbg_s.hole,
        w,
        h,
        &dbg_s.finest_owner,
        &dbg_s.finest_patches,
    );
    let m_naive = modeled_seam_energy(
        &naive.planes[0],
        &input,
        &dbg_n.hole,
        w,
        h,
        &dbg_n.finest_owner,
        &dbg_n.finest_patches,
    );
    assert!(
        m_seam < m_naive,
        "seam pass must lower boundary energy ({m_seam} vs {m_naive})"
    );
    assert_ne!(
        plane_hash(&with_seam.planes[0].data),
        plane_hash(&naive.planes[0].data),
        "seam pass changed nothing on the noise fixture"
    );
}

/// Decontamination reduces the boundary halo when the sample region's
/// statistics differ from the hole's surroundings (custom S forces the
/// fill to source from a darker textured region).
#[test]
fn decontamination_reduces_halo() {
    let (w, h) = (48, 24);
    let mut img = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            img.data[y * w + x] = if x < 24 {
                // Textured dark source region.
                if (x / 3) % 2 == 0 {
                    0.35
                } else {
                    0.45
                }
            } else {
                0.62
            };
        }
    }
    let hole = rect_mask(w, h, 30, 8, 40, 16);
    let sample = rect_mask(w, h, 0, 0, 24, h);
    corrupt(&mut img, &hole, 0.1);
    let planes = vec![img];
    let halo = |out: &Plane| -> f64 {
        let hb: Vec<bool> = hole.data.iter().map(|&v| v >= 0.5).collect();
        let mut acc = 0f64;
        let mut n = 0usize;
        for (i, &in_h) in hb.iter().enumerate() {
            if in_h {
                acc += out.data[i] as f64;
                n += 1;
            }
        }
        (acc / n as f64 - 0.62).abs()
    };
    let plain = exemplar_fill(&planes, None, &hole, Some(&sample), &no_adapt()).unwrap();
    let adapted = exemplar_fill(
        &planes,
        None,
        &hole,
        Some(&sample),
        &ExemplarParams::default(),
    )
    .unwrap();
    let h_plain = halo(&plain.planes[0]);
    let h_adapted = halo(&adapted.planes[0]);
    assert!(
        h_adapted < h_plain,
        "adaptation must shrink the halo ({h_adapted} vs {h_plain})"
    );
    assert!(h_plain > 0.05, "fixture failed to produce a halo");
}

/// §45.3 failure rule 1: a starved sample region (< 64 valid centers at
/// the coarsest level) degrades to boundary diffusion — the command
/// SUCCEEDS and reports the method.
#[test]
fn starved_sample_region_falls_back_to_diffusion() {
    let (w, h) = (40, 40);
    let hole = rect_mask(w, h, 15, 15, 25, 25);
    let mut img = stripes(w, h, 4);
    corrupt(&mut img, &hole, 0.5);
    // Tiny custom S: an 8×8 area yields (8−6)² = 4 < 64 centers.
    let sample = rect_mask(w, h, 0, 0, 8, 8);
    let out = exemplar_fill(
        &[img],
        None,
        &hole,
        Some(&sample),
        &ExemplarParams::default(),
    )
    .unwrap();
    assert_eq!(out.method, FillMethod::DiffusionFallback);
    // Pixels were produced (never a silent no-op): the object is gone.
    let center = out.planes[0].data[20 * w + 20];
    assert!(
        (center - 0.5).abs() > 1e-3,
        "hole center untouched: {center}"
    );
}

/// §45.3 failure rule 2: empty boundary + empty samples errors.
#[test]
fn all_hole_canvas_errors() {
    let (w, h) = (16, 16);
    let img = Plane::from_vec(w, h, vec![0.5; w * h]);
    let hole = rect_mask(w, h, 0, 0, w, h);
    let err = exemplar_fill(&[img], None, &hole, None, &ExemplarParams::default());
    assert_eq!(err.err(), Some(ExemplarError::SampleRegionEmpty));
}

/// Alpha-aware candidate validity: fully transparent pixels never source
/// a fill (their patches are invalid by construction).
#[test]
fn transparent_pixels_are_invalid_sources() {
    let (w, h) = (44, 44);
    let mut img = stripes(w, h, 4);
    // Right half: different (misleading) content, fully transparent.
    let mut alpha = Plane::from_vec(w, h, vec![1.0; w * h]);
    for y in 0..h {
        for x in 22..w {
            img.data[y * w + x] = 0.99;
            alpha.data[y * w + x] = 0.0;
        }
    }
    let hole = rect_mask(w, h, 8, 17, 16, 27);
    corrupt(&mut img, &hole, 0.5);
    let (out, dbg) = fill_with_knobs(
        &[img],
        Some(&alpha),
        &hole,
        None,
        &no_adapt(),
        Knobs::default(),
    )
    .unwrap();
    assert_eq!(out.method, FillMethod::Exemplar);
    for (i, s) in dbg.finest_src.iter().enumerate() {
        if dbg.hole[i] && s.is_some() {
            assert!(
                (s.x as usize) < 22,
                "pixel {i} sourced from the transparent region (x = {})",
                s.x
            );
        }
    }
}

/// The mirror library participates and wins where only mirrored content
/// continues the pattern: the right half is the horizontal mirror of the
/// left, S is restricted to the left half (wrong chirality for the
/// right-half hole), and enabling `mirror` must lower the error.
#[test]
fn mirror_library_participates() {
    let (w, h) = (48, 32);
    let mut truth = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let sx = if x < 24 { x } else { w - 1 - x };
            truth.data[y * w + x] = ((2 * sx + 3 * y) % 11) as f32 / 11.0;
        }
    }
    let hole = rect_mask(w, h, 30, 10, 40, 22);
    let sample = rect_mask(w, h, 0, 0, 24, h);
    let mut img = truth.clone();
    corrupt(&mut img, &hole, 0.5);
    let planes = vec![img];
    let base = exemplar_fill(&planes, None, &hole, Some(&sample), &no_adapt()).unwrap();
    let mirrored = exemplar_fill(
        &planes,
        None,
        &hole,
        Some(&sample),
        &ExemplarParams {
            mirror: true,
            ..no_adapt()
        },
    )
    .unwrap();
    assert_eq!(mirrored.dropped_libraries, 0, "mirror must fit the cap");
    assert_ne!(
        plane_hash(&base.planes[0].data),
        plane_hash(&mirrored.planes[0].data),
        "mirror library never participated"
    );
    let err_base = interior_err(&base.planes[0], &truth, &hole);
    let err_mirror = interior_err(&mirrored.planes[0], &truth, &hole);
    assert!(
        err_mirror < err_base,
        "mirrored library must win on the chirality fixture ({err_mirror} vs {err_base})"
    );
}

/// Multi-level pyramid path (64×64 → two levels at floor 32) is exercised
/// and deterministic, and the periodic oracle still holds.
#[test]
fn multi_level_pyramid_fill() {
    let (w, h) = (64, 64);
    let truth = stripes(w, h, 4);
    let hole = rect_mask(w, h, 26, 26, 38, 38);
    let mut img = truth.clone();
    corrupt(&mut img, &hole, 0.5);
    let planes = vec![img];
    let a = exemplar_fill(&planes, None, &hole, None, &no_adapt()).unwrap();
    let b = exemplar_fill(&planes, None, &hole, None, &no_adapt()).unwrap();
    assert_eq!(plane_hash(&a.planes[0].data), plane_hash(&b.planes[0].data));
    let err = interior_err(&a.planes[0], &truth, &hole);
    assert!(err < 0.08, "multi-level stripe error {err}");
}

/// §45.3 Content-Aware Move parameter mapping.
#[test]
fn content_aware_move_mapping() {
    let p = ExemplarParams::for_content_aware_move(4, 5);
    assert!((p.kappa - 0.65).abs() < 1e-12);
    assert_eq!(
        p.color_adaptation,
        ColorAdaptation::Global { strength: 0.5 }
    );
    // Clamping.
    let q = ExemplarParams::for_content_aware_move(0, 99);
    assert!((q.kappa - 0.95).abs() < 1e-12);
    assert_eq!(
        q.color_adaptation,
        ColorAdaptation::Global { strength: 1.0 }
    );
}

// ------------------------------------------------------------------ goldens

/// Pinned goldens (own runs, recorded AFTER the behavioral oracles above
/// were green — hand-invented hashes are banned). Any drift is an
/// investigation, not a re-pin (O-QA rule).
#[test]
fn pinned_goldens() {
    // Stripes, default params, seed 0.
    let hole = rect_mask(44, 44, 17, 17, 27, 27);
    let mut img = stripes(44, 44, 4);
    corrupt(&mut img, &hole, 0.5);
    let out = exemplar_fill(&[img], None, &hole, None, &ExemplarParams::default()).unwrap();
    eprintln!("golden stripes seed0: {}", plane_hash(&out.planes[0].data));
    let g_stripes = plane_hash(&out.planes[0].data);

    // The tie-band fixture, seeds 0 and 1 (tie-rich; distinct by the
    // §45.3 seed policy).
    let (planes, hole) = tie_band_fixture();
    let s0 = exemplar_fill(
        &planes,
        None,
        &hole,
        None,
        &ExemplarParams {
            seed: 0,
            ..no_adapt()
        },
    )
    .unwrap();
    let s1 = exemplar_fill(
        &planes,
        None,
        &hole,
        None,
        &ExemplarParams {
            seed: 1,
            ..no_adapt()
        },
    )
    .unwrap();
    eprintln!("golden tie-band seed0: {}", plane_hash(&s0.planes[0].data));
    eprintln!("golden tie-band seed1: {}", plane_hash(&s1.planes[0].data));
    let g_c0 = plane_hash(&s0.planes[0].data);
    let g_c1 = plane_hash(&s1.planes[0].data);

    // Scale-library cap fixture (fix-round defect 4): `scale + mirror`
    // on the stripes fixture exceeds the 2×-base materialization cap, so
    // the §45.3 coarsest-first drop rule participates in this golden.
    // Pinned AFTER the transform_cap_drops_coarsest_scale_first oracle
    // was green, from two-run-equal own output.
    let hole = rect_mask(44, 44, 17, 17, 27, 27);
    let mut img = stripes(44, 44, 4);
    corrupt(&mut img, &hole, 0.5);
    let sc = exemplar_fill(
        &[img],
        None,
        &hole,
        None,
        &ExemplarParams {
            scale: true,
            mirror: true,
            ..no_adapt()
        },
    )
    .unwrap();
    assert!(
        sc.dropped_libraries > 0,
        "cap fixture must actually hit the cap"
    );
    eprintln!(
        "golden scale-cap: {} (dropped {})",
        plane_hash(&sc.planes[0].data),
        sc.dropped_libraries
    );
    let g_sc = plane_hash(&sc.planes[0].data);

    // Starved fallback (diffusion path golden'd separately, §45.8 note).
    let hole = rect_mask(40, 40, 15, 15, 25, 25);
    let mut img = stripes(40, 40, 4);
    corrupt(&mut img, &hole, 0.5);
    let sample = rect_mask(40, 40, 0, 0, 8, 8);
    let fb = exemplar_fill(
        &[img],
        None,
        &hole,
        Some(&sample),
        &ExemplarParams::default(),
    )
    .unwrap();
    eprintln!("golden fallback: {}", plane_hash(&fb.planes[0].data));
    let g_fb = plane_hash(&fb.planes[0].data);
    assert_eq!(g_stripes, GOLDEN_STRIPES_SEED0);
    assert_eq!(g_c0, GOLDEN_TIE_BAND_SEED0);
    assert_eq!(g_c1, GOLDEN_TIE_BAND_SEED1);
    assert_ne!(g_c0, g_c1, "seed goldens collapsed");
    assert_eq!(g_fb, GOLDEN_FALLBACK);
    assert_eq!(g_sc, GOLDEN_SCALE_CAP);
}

// ------------------------------------------- tester round (Row A, §5.3)
// Tests the builder didn't think of: mask/sample-region edge cases,
// preset relaxations, transform-allowance behavior, and input-policy
// pins. Implementation untouched (tester charter); every fixture claim
// is a real oracle or an honestly-labeled behavior pin.

/// Hole touching all four canvas borders (a full-span cross): the
/// priority front starts with border-clipped patches on every arm and
/// the front normal is degenerate at the canvas edge. The engine must
/// run the exemplar path, terminate, continue the periodic texture in
/// the interior, and stay two-run byte-identical.
#[test]
fn hole_touching_all_four_borders_fills() {
    let (w, h) = (48, 48);
    let truth = stripes(w, h, 4);
    let mut hole = rect_mask(w, h, 0, 20, w, 28); // horizontal band, full width
    for y in 0..h {
        for x in 20..28 {
            hole.data[y * w + x] = 1.0; // vertical band, full height
        }
    }
    let mut img = truth.clone();
    corrupt(&mut img, &hole, 0.5);
    let planes = vec![img];
    let a = exemplar_fill(&planes, None, &hole, None, &no_adapt()).unwrap();
    let b = exemplar_fill(&planes, None, &hole, None, &no_adapt()).unwrap();
    assert_eq!(a.method, FillMethod::Exemplar);
    assert_eq!(plane_hash(&a.planes[0].data), plane_hash(&b.planes[0].data));
    let err = interior_err(&a.planes[0], &truth, &hole);
    eprintln!("cross-hole interior err: {err}");
    // Measured 0.0303; bound set with honest margin above measurement.
    assert!(err < 0.1, "border-touching cross continuation error {err}");
}

/// §45.3 failure rule 1 BOUNDARY: N_min = 64 valid centers at the
/// coarsest level is the exact viability threshold ("< 64" is starved).
/// A 14×14 corner S yields exactly (14−6)² = 64 centers → exemplar
/// path; a 15×13 S yields 9·7 = 63 → diffusion fallback; an S smaller
/// than one 7×7 patch yields 0 → fallback. Single-level canvas so the
/// finest-level count IS the coarsest-level count.
#[test]
fn n_min_boundary_exactly_64_centers() {
    let (w, h) = (40, 40);
    let hole = rect_mask(w, h, 20, 20, 30, 30);
    let mut img = stripes(w, h, 4);
    corrupt(&mut img, &hole, 0.5);
    let planes = vec![img];
    let run = |sample: &Plane| {
        exemplar_fill(&planes, None, &hole, Some(sample), &no_adapt())
            .unwrap()
            .method
    };
    // Exactly 64 centers: NOT starved (the rule is strict `< 64`).
    assert_eq!(
        run(&rect_mask(w, h, 0, 0, 14, 14)),
        FillMethod::Exemplar,
        "exactly N_min centers must take the exemplar path"
    );
    // 63 centers: starved.
    assert_eq!(
        run(&rect_mask(w, h, 0, 0, 15, 13)),
        FillMethod::DiffusionFallback,
        "N_min − 1 centers must degrade to diffusion"
    );
    // S smaller than one patch: zero centers, starved.
    assert_eq!(
        run(&rect_mask(w, h, 0, 0, 6, 6)),
        FillMethod::DiffusionFallback,
        "sub-patch S must degrade to diffusion"
    );
}

/// Two disjoint holes: one priority front serves both regions and the
/// pop interleaving is fully determined by the total order (priority
/// bits, seeded tie, y, x) — two runs are byte-identical and both
/// interiors continue the texture.
#[test]
fn two_disjoint_holes_fill_deterministically() {
    let (w, h) = (44, 44);
    let truth = stripes(w, h, 4);
    let mut hole = rect_mask(w, h, 8, 8, 16, 16);
    for y in 28..36 {
        for x in 28..36 {
            hole.data[y * w + x] = 1.0;
        }
    }
    let mut img = truth.clone();
    corrupt(&mut img, &hole, 0.5);
    let planes = vec![img];
    let a = exemplar_fill(&planes, None, &hole, None, &no_adapt()).unwrap();
    let b = exemplar_fill(&planes, None, &hole, None, &no_adapt()).unwrap();
    assert_eq!(a.method, FillMethod::Exemplar);
    assert_eq!(
        plane_hash(&a.planes[0].data),
        plane_hash(&b.planes[0].data),
        "interleaved fronts must be deterministic"
    );
    let err = interior_err(&a.planes[0], &truth, &hole);
    eprintln!("two-hole interior err: {err}");
    // Measured 0.0 exactly (both interiors reproduce the stripes).
    assert!(err < 0.05, "two-hole continuation error {err}");
}

/// 1-px-wide serpentine hole: every hole pixel is front from the start
/// (maximal front degeneracy — the normal estimate can vanish and the
/// patch-area confidence is nearly saturated everywhere). The engine
/// must terminate, commit every pixel, and stay deterministic. NOTE
/// (spec-faithful): a 1-px hole lies entirely inside the §45.3 2-px
/// feather band (depth 1, coverage smoothstep(1/3) ≈ 0.259), so the
/// output is dominated by the original (corrupt-to-0) content by
/// design; the fill's contribution still must move every pixel
/// strictly off the corrupt value.
#[test]
fn serpentine_one_pixel_hole_terminates() {
    let (w, h) = (40, 40);
    let truth = stripes(w, h, 4);
    let mut hole = Plane::new(w, h);
    for (r, row) in [8usize, 14, 20, 26].iter().enumerate() {
        for x in 4..36 {
            hole.data[row * w + x] = 1.0;
        }
        // Alternating end connectors keep it one serpentine path.
        if r < 3 {
            let x = if r % 2 == 0 { 35 } else { 4 };
            for y in row + 1..row + 6 {
                hole.data[y * w + x] = 1.0;
            }
        }
    }
    let mut img = truth.clone();
    corrupt(&mut img, &hole, 0.0);
    let planes = vec![img];
    let a = exemplar_fill(&planes, None, &hole, None, &no_adapt()).unwrap();
    let b = exemplar_fill(&planes, None, &hole, None, &no_adapt()).unwrap();
    assert_eq!(a.method, FillMethod::Exemplar);
    assert_eq!(plane_hash(&a.planes[0].data), plane_hash(&b.planes[0].data));
    let hb: Vec<bool> = hole.data.iter().map(|&v| v >= 0.5).collect();
    let mut lo = f32::INFINITY;
    for (i, &in_h) in hb.iter().enumerate() {
        if in_h {
            let v = a.planes[0].data[i];
            assert!(v.is_finite() && (0.0..=1.0).contains(&v));
            lo = lo.min(v);
        }
    }
    eprintln!("serpentine min hole value: {lo}");
    assert!(
        lo > 0.03,
        "a serpentine pixel kept the corrupt value: min {lo}"
    );
}

/// A hole so large that the AUTO sample region starves at the coarsest
/// level (96×96 canvas, 70×70 hole: at the 48×48 coarsest level the
/// remaining frame minus the 2-px guard band is thinner than one 7×7
/// patch — zero valid centers < N_min): §45.3 failure rule 1 applies
/// even with `sample = None`, and the diffusion degradation must still
/// produce pixels deterministically.
#[test]
fn oversized_hole_starves_auto_sample_region() {
    let (w, h) = (96, 96);
    let hole = rect_mask(w, h, 13, 13, 83, 83);
    let mut img = stripes(w, h, 4);
    corrupt(&mut img, &hole, 0.5);
    let planes = vec![img];
    let a = exemplar_fill(&planes, None, &hole, None, &ExemplarParams::default()).unwrap();
    let b = exemplar_fill(&planes, None, &hole, None, &ExemplarParams::default()).unwrap();
    assert_eq!(a.method, FillMethod::DiffusionFallback);
    assert_eq!(plane_hash(&a.planes[0].data), plane_hash(&b.planes[0].data));
    let center = a.planes[0].data[48 * w + 48];
    assert!(
        (center - 0.5).abs() > 1e-3,
        "hole center untouched: {center}"
    );
}

/// Custom S disjoint from the hole's surroundings: coherence candidates
/// are starved of locally-plausible sources and every index candidate
/// comes from content unlike the boundary. The engine must terminate
/// deterministically on the exemplar path, and every committed source
/// must lie inside S (provenance oracle over the debug SrcRefs).
#[test]
fn disjoint_sample_region_sources_only_from_s() {
    let (w, h) = (48, 24);
    let mut img = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            img.data[y * w + x] = if x < 20 {
                // S texture, unlike anything near the hole.
                if ((x / 3) + (y / 3)) % 2 == 0 {
                    0.3
                } else {
                    0.6
                }
            } else {
                0.85
            };
        }
    }
    let hole = rect_mask(w, h, 30, 8, 40, 16);
    let sample = rect_mask(w, h, 0, 0, 20, h);
    corrupt(&mut img, &hole, 0.1);
    let planes = vec![img];
    let (a, dbg) = fill_with_knobs(
        &planes,
        None,
        &hole,
        Some(&sample),
        &no_adapt(),
        Knobs::default(),
    )
    .unwrap();
    let (b, _) = fill_with_knobs(
        &planes,
        None,
        &hole,
        Some(&sample),
        &no_adapt(),
        Knobs::default(),
    )
    .unwrap();
    assert_eq!(a.method, FillMethod::Exemplar);
    assert_eq!(plane_hash(&a.planes[0].data), plane_hash(&b.planes[0].data));
    let mut committed = 0usize;
    for (i, s) in dbg.finest_src.iter().enumerate() {
        if dbg.hole[i] && s.is_some() {
            committed += 1;
            assert_eq!(s.lib, 0, "identity library only");
            assert!(
                (s.x as usize) < 20,
                "pixel {i} sourced outside S (x = {})",
                s.x
            );
        }
    }
    assert!(committed > 0, "no committed sources to check");
}

/// §21.4 draft relaxations are live: (1) draft runs NO color adaptation
/// — a draft fill with the default global adaptation is byte-identical
/// to a draft fill with adaptation disabled, on a fixture where the
/// default preset provably adapts; (2) the draft schedule (floor 64,
/// top-4, R = 0) actually changes the computation vs the default preset
/// on a stochastic fixture.
#[test]
fn draft_relaxations_hold() {
    // Fixture from the decontamination oracle: S statistics differ from
    // the hole surroundings, so adaptation has something to do.
    let (w, h) = (48, 24);
    let mut img = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            img.data[y * w + x] = if x < 24 {
                if (x / 3) % 2 == 0 {
                    0.35
                } else {
                    0.45
                }
            } else {
                0.62
            };
        }
    }
    let hole = rect_mask(w, h, 30, 8, 40, 16);
    let sample = rect_mask(w, h, 0, 0, 24, h);
    corrupt(&mut img, &hole, 0.1);
    let planes = vec![img];
    let draft_adapted = exemplar_fill(
        &planes,
        None,
        &hole,
        Some(&sample),
        &ExemplarParams {
            quality: Quality::Draft,
            ..ExemplarParams::default()
        },
    )
    .unwrap();
    let draft_plain = exemplar_fill(
        &planes,
        None,
        &hole,
        Some(&sample),
        &ExemplarParams {
            quality: Quality::Draft,
            ..no_adapt()
        },
    )
    .unwrap();
    assert_eq!(
        plane_hash(&draft_adapted.planes[0].data),
        plane_hash(&draft_plain.planes[0].data),
        "draft must ignore colorAdaptation entirely"
    );
    // Control: at default quality the same two param sets DIFFER.
    let def_adapted = exemplar_fill(
        &planes,
        None,
        &hole,
        Some(&sample),
        &ExemplarParams::default(),
    )
    .unwrap();
    let def_plain = exemplar_fill(&planes, None, &hole, Some(&sample), &no_adapt()).unwrap();
    assert_ne!(
        plane_hash(&def_adapted.planes[0].data),
        plane_hash(&def_plain.planes[0].data),
        "control: default preset must adapt on this fixture"
    );
    // The draft schedule itself differs from the default schedule.
    let (w2, h2) = (48, 40);
    let mut nimg = noise(w2, h2, 0xD4A);
    let nhole = rect_mask(w2, h2, 18, 14, 32, 26);
    corrupt(&mut nimg, &nhole, 0.5);
    let nplanes = vec![nimg];
    let d = exemplar_fill(
        &nplanes,
        None,
        &nhole,
        None,
        &ExemplarParams {
            quality: Quality::Draft,
            ..no_adapt()
        },
    )
    .unwrap();
    let q = exemplar_fill(&nplanes, None, &nhole, None, &no_adapt()).unwrap();
    assert_ne!(
        plane_hash(&d.planes[0].data),
        plane_hash(&q.planes[0].data),
        "draft and default schedules coincided on a noise fixture"
    );
}

/// §45.3 Content-Aware Move mapping ENDPOINTS + non-mapped defaults:
/// κ = 1.05 − 0.1·structure over the full 1–7 range, w = color/10 at
/// both ends, and every field outside the mapping keeps its default
/// (the existing mapping test covers midpoints and clamping).
#[test]
fn content_aware_move_mapping_endpoints_and_defaults() {
    let lo = ExemplarParams::for_content_aware_move(1, 0);
    assert!((lo.kappa - 0.95).abs() < 1e-12);
    assert_eq!(
        lo.color_adaptation,
        ColorAdaptation::Global { strength: 0.0 }
    );
    let hi = ExemplarParams::for_content_aware_move(7, 10);
    assert!((hi.kappa - 0.35).abs() < 1e-12);
    assert_eq!(
        hi.color_adaptation,
        ColorAdaptation::Global { strength: 1.0 }
    );
    // Non-mapped fields stay at engine defaults.
    assert_eq!(lo.quality, Quality::Default);
    assert_eq!(lo.seed, 0);
    assert_eq!(lo.rotation, RotationAdaptation::None);
    assert!(!lo.scale && !lo.mirror);
}

/// Rotated-texture fixture for the §45.3 rotation allowances: S (a left
/// strip) carries vertical stripes; the truth around the hole is the
/// same texture rotated by 30° about the canvas center — inside Full's
/// θ = k·15° set, outside Low's ±15° set. Full must actually commit
/// from a rotated library and continue the rotated texture better than
/// Low and None (quantified below from measured runs).
#[test]
fn rotation_full_finds_rotated_continuation() {
    let (w, h) = (44, 44);
    let (cx, cy) = ((w as f64 - 1.0) * 0.5, (h as f64 - 1.0) * 0.5);
    let stripe_val = |t: f64| -> f32 {
        // Period-8 stripes over the rotated coordinate, phased at the
        // canvas center.
        if ((t + 1024.0) / 4.0) as i64 % 2 == 0 {
            0.25
        } else {
            0.75
        }
    };
    let (sin30, cos30) = 30f64.to_radians().sin_cos();
    let mut truth = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let u = if x < 14 {
                x as f64 - cx // S strip: unrotated vertical stripes
            } else {
                cos30 * (x as f64 - cx) + sin30 * (y as f64 - cy)
            };
            truth.data[y * w + x] = stripe_val(u);
        }
    }
    let hole = rect_mask(w, h, 24, 16, 36, 28);
    let sample = rect_mask(w, h, 0, 0, 14, h);
    let mut img = truth.clone();
    corrupt(&mut img, &hole, 0.5);
    let planes = vec![img];
    let run = |rotation: RotationAdaptation| {
        fill_with_knobs(
            &planes,
            None,
            &hole,
            Some(&sample),
            &ExemplarParams {
                rotation,
                ..no_adapt()
            },
            Knobs::default(),
        )
        .unwrap()
    };
    let (none, _) = run(RotationAdaptation::None);
    let (low, _) = run(RotationAdaptation::Low);
    let (full, dbg) = run(RotationAdaptation::Full);
    let rotated_commits = dbg
        .finest_src
        .iter()
        .enumerate()
        .filter(|&(i, s)| dbg.hole[i] && s.is_some() && s.lib > 0)
        .count();
    let err_none = interior_err(&none.planes[0], &truth, &hole);
    let err_low = interior_err(&low.planes[0], &truth, &hole);
    let err_full = interior_err(&full.planes[0], &truth, &hole);
    eprintln!(
        "rotation errs: none {err_none} low {err_low} full {err_full}; rotated commits {rotated_commits}"
    );
    assert!(
        rotated_commits > 0,
        "Full never committed from a rotated library"
    );
    assert!(
        err_full < err_low && err_full < err_none,
        "Full ({err_full}) must beat Low ({err_low}) and None ({err_none})"
    );
    // Measured: none 0.305, low 0.225, full 0.047 (137 rotated-library
    // commits) — Full is ~5–6× better; bound with honest margin.
    assert!(err_full < 0.1, "rotated continuation error {err_full}");
}

/// NaN input-policy pins (documents current behavior, not a contract;
/// the adapter seam performs no pixel-data validation).
///
/// (1) NaN INSIDE the hole (single-level canvas): the §45.3 stages
/// overwrite hole content (diffusion init + commits), so synthesis is
/// unaffected — but the output feather composite is the lerp form
/// `orig + (fill − orig) · cov`, which references the ORIGINAL buffer
/// at EVERY coverage (including cov = 1), so the NaN re-enters across
/// the whole hole, not just the 2-px band (measured: every hole pixel
/// NaN, every known pixel untouched). Deterministic per run
/// (bit-identical hashes).
/// (2) NaN in the SAMPLE region: poisons descriptor/SSD arithmetic;
/// the engine still terminates and is two-run bit-identical (total
/// orders over payload bits — no NaN-driven nondeterminism).
/// (3) NaN in the HOLE MASK plane: `NaN >= 0.5` is false, so the pixel
/// is simply not part of the hole and its input value passes through
/// bit-exactly.
#[test]
fn nan_input_policy_pinned() {
    // (1) NaN inside the hole.
    let (w, h) = (40, 40);
    let hole = rect_mask(w, h, 15, 15, 25, 25);
    let mut img = stripes(w, h, 4);
    corrupt(&mut img, &hole, f32::NAN);
    let planes = vec![img];
    let a = exemplar_fill(&planes, None, &hole, None, &no_adapt()).unwrap();
    let b = exemplar_fill(&planes, None, &hole, None, &no_adapt()).unwrap();
    assert_eq!(plane_hash(&a.planes[0].data), plane_hash(&b.planes[0].data));
    let hb: Vec<bool> = hole.data.iter().map(|&v| v >= 0.5).collect();
    for (i, (&in_h, &v)) in hb.iter().zip(&a.planes[0].data).enumerate() {
        if in_h {
            assert!(v.is_nan(), "hole pixel {i} lost the NaN pin: {v}");
        } else {
            assert!(v.is_finite(), "known pixel {i} became {v}");
        }
    }
    // (2) NaN in the sample region (outside hole + guard).
    let mut img2 = stripes(w, h, 4);
    img2.data[5 * w + 5] = f32::NAN;
    corrupt(&mut img2, &hole, 0.5);
    let planes2 = vec![img2];
    let a2 = exemplar_fill(&planes2, None, &hole, None, &no_adapt()).unwrap();
    let b2 = exemplar_fill(&planes2, None, &hole, None, &no_adapt()).unwrap();
    assert_eq!(
        plane_hash(&a2.planes[0].data),
        plane_hash(&b2.planes[0].data),
        "NaN in S must not break two-run bit identity"
    );
    // (3) NaN in the hole mask: not-a-hole, value passes through.
    let mut hole_nan = hole.clone();
    hole_nan.data[20 * w + 20] = f32::NAN;
    let mut img3 = stripes(w, h, 4);
    corrupt(&mut img3, &hole_nan, 0.5); // NaN-mask pixel keeps 0.5? no — corrupt tests >= 0.5, NaN false → keeps stripe value
    let keep = img3.data[20 * w + 20];
    let planes3 = vec![img3];
    let a3 = exemplar_fill(&planes3, None, &hole_nan, None, &no_adapt()).unwrap();
    assert_eq!(
        a3.planes[0].data[20 * w + 20].to_bits(),
        keep.to_bits(),
        "NaN mask pixel must pass its input value through bit-exactly"
    );
}

// Pinned 2026-08-16 from this suite's own runs after the oracles above
// were green (see the lane log Row A block for the evidence chain).
const GOLDEN_STRIPES_SEED0: u64 = 5242724163736498141;
const GOLDEN_TIE_BAND_SEED0: u64 = 2086179200194383749;
const GOLDEN_TIE_BAND_SEED1: u64 = 2100125350095197365;
const GOLDEN_FALLBACK: u64 = 16729089110742476051;
// Pinned 2026-08-16 (fix round) from two-run-equal own output after the
// transform-cap drop-priority oracle was green (see the fix-round log
// entry); this golden covers the §45.3 coarsest-first drop rule.
const GOLDEN_SCALE_CAP: u64 = 2264002749406661101;
