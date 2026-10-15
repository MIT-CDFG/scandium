// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The §34 parity row for `channel.apply_image`/`channel.calculations`
//! (O-INT CP-1.0 dispatch; ES §14.4 + §34.3; plan §3.4 "their parity
//! harness is yours"): appname-color's apply doors consumed with THIS
//! crate's exported §14.4 seam kernels —
//!
//! 1. CPU end-to-end bit-identity (always runs): at the neutral shape
//!    (opacity 1, scale 1, offset 0, no invert, no coverage planes) the
//!    door hands back the kernel's value BIT-FOR-BIT per its own w = 1
//!    contract — the one-math-library clause exercised through the real
//!    consumer path for every kernel-bearing mode.
//! 2. GPU/CPU parity at the equivalence point (adapter-gated, W24 skip
//!    semantics as in gpu_parity.rs): §14.4 at the neutral shape
//!    computes R = B(T, S) per §4.4's opaque-composite reduction
//!    (αs = αb = 1 ⇒ Co = B exactly, term-by-term in f32), so the
//!    plane-wise CPU application must match the GPU kernel's RGB within
//!    the §34.3 ≤ 1/255 gate. Domain: [0, 1] planes (S″'s clamp is the
//!    §14.4 conditioning; HDR apply-image inputs condition to [0, 1] by
//!    spec, so the equivalence point is the whole §14.4 domain).
//!
//! Dissolve is excluded by construction: the seam exports nothing for
//! it (color-side `dissolve_reserved`, fold-in record in the O-COLOR
//! log) — asserted here so the exclusion stays typed, never silent.

use appname_color::apply_image::{apply_blend_plane, apply_blend_planes3, ApplyShape, CoverageArm};
use appname_core::{BlendMode, TILE};
use appname_render::{
    render_separable_kernel, render_triple_kernel, CompositeBackend, GpuBackend, GpuUnavailable,
    KernelCase, TileRgba,
};

const N: usize = TILE * TILE;

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// Seeded [0, 1] plane with the §14.4-relevant edge values salted in
/// (exact 0/1, dyadics, near-threshold values for the step modes).
fn plane(seed: u64) -> Vec<f32> {
    let mut rng = seed;
    let mut out: Vec<f32> = (0..N)
        .map(|_| (splitmix(&mut rng) % 4097) as f32 / 4096.0)
        .collect();
    let salts = [0.0f32, 1.0, 0.5, 0.25, 0.49999997, 0.50000006];
    for (i, v) in salts.iter().enumerate() {
        out[i * 97] = *v;
    }
    out
}

fn triple_modes() -> [BlendMode; 6] {
    [
        BlendMode::DarkerColor,
        BlendMode::LighterColor,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ]
}

/// Leg 1 — CPU end-to-end bit-identity through color's doors at the
/// neutral shape, every kernel-bearing mode.
#[test]
fn apply_image_neutral_shape_is_kernel_exact_cpu() {
    let shape = ApplyShape::default();
    let cov = CoverageArm::default();
    let (src, tgt0) = (plane(0xA1), plane(0xB2));

    let mut separable = 0;
    for mode in BlendMode::ALL {
        let Some(kernel) = render_separable_kernel(mode) else {
            continue;
        };
        separable += 1;
        let mut tgt = tgt0.clone();
        apply_blend_plane(kernel, &mut tgt, &src, &shape, &cov).expect("door accepts");
        for i in 0..N {
            // S″ at neutral = clamp(s, 0, 1) = s on [0, 1] planes.
            let want = kernel(tgt0[i], src[i]);
            assert_eq!(
                tgt[i].to_bits(),
                want.to_bits(),
                "{mode:?} @{i}: door must hand back the kernel bit-for-bit at w = 1"
            );
        }
    }
    assert_eq!(separable, 20);

    let (sr, sg, sb) = (plane(0xC1), plane(0xC2), plane(0xC3));
    let (tr0, tg0, tb0) = (plane(0xD1), plane(0xD2), plane(0xD3));
    for mode in triple_modes() {
        let kernel = render_triple_kernel(mode).expect("triple arm");
        let (mut tr, mut tg, mut tb) = (tr0.clone(), tg0.clone(), tb0.clone());
        apply_blend_planes3(
            kernel,
            [&mut tr, &mut tg, &mut tb],
            [&sr, &sg, &sb],
            &shape,
            &cov,
        )
        .expect("door accepts");
        for i in 0..N {
            let want = kernel([tr0[i], tg0[i], tb0[i]], [sr[i], sg[i], sb[i]]);
            for (c, got) in [tr[i], tg[i], tb[i]].into_iter().enumerate() {
                assert_eq!(
                    got.to_bits(),
                    want[c].to_bits(),
                    "{mode:?} @{i} ch{c}: whole-pixel/non-sep triple must survive w = 1 bit-for-bit"
                );
            }
        }
    }

    // Dissolve: excluded by construction — the seam exports nothing.
    assert!(render_separable_kernel(BlendMode::Dissolve).is_none());
    assert!(render_triple_kernel(BlendMode::Dissolve).is_none());
}

/// Leg 2 — GPU/CPU parity at the §14.4/§4.4 equivalence point: the
/// door's plane output vs the GPU kernel's RGB over opaque tiles,
/// ≤ 1/255 (§34.3), every kernel-bearing mode. Adapter-gated with the
/// gpu_parity.rs skip semantics (W24: absence skips; present-but-broken
/// fails; APPNAME_REQUIRE_ADAPTER hardens absence).
#[test]
fn apply_image_gpu_cpu_parity_at_equivalence_point() {
    let mut gpu = match GpuBackend::new() {
        Ok(g) => g,
        Err(GpuUnavailable::NoAdapter) => {
            if std::env::var_os("APPNAME_REQUIRE_ADAPTER").is_some() {
                panic!("APPNAME_REQUIRE_ADAPTER set but no adapter found");
            }
            eprintln!("apply_image parity: no adapter — CPU path active (W24)");
            return;
        }
        Err(state) => panic!("adapter present but backend init failed: {state:?}"),
    };

    let (sr, sg, sb) = (plane(0xE1), plane(0xE2), plane(0xE3));
    let (tr0, tg0, tb0) = (plane(0xF1), plane(0xF2), plane(0xF3));
    // Opaque tiles carrying the plane values (αs = αb = 1 ⇒ Co = B).
    let mut src_tile = TileRgba::transparent();
    let mut dst_tile = TileRgba::transparent();
    for i in 0..N {
        let (x, y) = (i % TILE, i / TILE);
        src_tile.set(x, y, [sr[i], sg[i], sb[i], 1.0]);
        dst_tile.set(x, y, [tr0[i], tg0[i], tb0[i], 1.0]);
    }

    let shape = ApplyShape::default();
    let cov = CoverageArm::default();
    let mut worst: (f32, &'static str) = (0.0, "none");
    for mode in BlendMode::ALL {
        if mode == BlendMode::Dissolve {
            continue; // typed-reserved color-side; nothing to compare
        }
        // CPU §14.4 application through color's doors.
        let (mut tr, mut tg, mut tb) = (tr0.clone(), tg0.clone(), tb0.clone());
        if let Some(kernel) = render_separable_kernel(mode) {
            apply_blend_plane(kernel, &mut tr, &sr, &shape, &cov).unwrap();
            apply_blend_plane(kernel, &mut tg, &sg, &shape, &cov).unwrap();
            apply_blend_plane(kernel, &mut tb, &sb, &shape, &cov).unwrap();
        } else {
            let kernel = render_triple_kernel(mode).expect("partition totality");
            apply_blend_planes3(
                kernel,
                [&mut tr, &mut tg, &mut tb],
                [&sr, &sg, &sb],
                &shape,
                &cov,
            )
            .unwrap();
        }
        // GPU composite at the equivalence point.
        let case = KernelCase {
            name: format!("apply-image/{mode:?}"),
            mode,
            src: src_tile.clone(),
            dst: dst_tile.clone(),
            opacity: 1.0,
            ctx: Default::default(),
            coverage: None,
        };
        let out = gpu.composite(&case);
        let mut max = 0.0f32;
        for i in 0..N {
            let (x, y) = (i % TILE, i / TILE);
            let px = out.get(x, y);
            for (got, want) in px[..3].iter().zip([tr[i], tg[i], tb[i]]) {
                let d = (got - want).abs() * 255.0;
                let d = if d.is_finite() { d } else { f32::INFINITY };
                max = max.max(d);
            }
            assert_eq!(px[3], 1.0, "{mode:?}: opaque-over-opaque alpha");
        }
        assert!(
            max <= 1.0,
            "{mode:?}: apply_image CPU vs GPU kernel diverged {max}/255 at the equivalence point"
        );
        if max > worst.0 {
            worst = (max, "");
        }
        eprintln!("apply_image parity {mode:?}: max_abs_8bit {max:.5}");
    }
    eprintln!(
        "apply_image parity [{}]: worst max_abs_8bit {:.5} over 26 modes",
        CompositeBackend::name(&gpu),
        worst.0
    );
}
