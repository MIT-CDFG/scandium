// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! R1 ruling arm 3 (O-INT, 2026-08-16; reopened-CP-0.7 §12.1 review):
//! PIN the alpha-preservation property of the §17 adjustment kernels —
//! per REGISTERED adjustment op in the shipped assembly, `apply_cpu`
//! must return each pixel's alpha bitwise-unchanged. The chain-edit
//! dirty math's masked arm (below-stack extent as a conservative
//! superset — `KindContext::note_op_composite_dirty`) is sound ONLY
//! while adjustments never write alpha; a future alpha-writing op must
//! turn THIS test red rather than silently invalidating that region
//! math. Spec: ES §17.1 (an adjustment is a pure function in the
//! working space — color, not coverage), §16.1 evaluation-domain
//! discipline (un-premultiply → apply → re-premultiply).
//!
//! Params: each op's registered default set (`default_params()` = `{}`,
//! every §17.2 param carries its block default — the same values
//! `adjustment.add` with `params: {}` commits). Property, not golden:
//! the pin is on ALPHA only, for whatever ops the assembly registers —
//! a newly registered adjustment joins automatically.

use appname_conformance::assembly_point::assembled_engine;
use appname_core::IRect;

#[test]
fn every_registered_adjustment_kernel_preserves_alpha_bitwise() {
    let engine = assembled_engine();
    let ops = engine.assembly().ops();
    let adjustments: Vec<&str> = ops
        .ids()
        .filter(|id| id.starts_with("adjustment."))
        .collect();
    assert!(
        !adjustments.is_empty(),
        "the shipped assembly registers the §17 corpus (stage guard)"
    );

    // Premultiplied probe plane: assorted alphas incl. the exact 0 and 1
    // ends, colors in premul gamut (c ≤ a), plus a gray ramp.
    let rect = IRect::new(0, 0, 4, 2);
    let input: [[f32; 4]; 8] = [
        [0.0, 0.0, 0.0, 0.0],
        [0.05, 0.1, 0.2, 0.25],
        [0.25, 0.125, 0.0625, 0.5],
        [0.9, 0.5, 0.1, 1.0],
        [0.75, 0.75, 0.75, 0.75],
        [0.33, 0.33, 0.33, 0.33],
        [1.0, 1.0, 1.0, 1.0],
        [0.001, 0.0005, 0.002, 0.125],
    ];

    for id in adjustments {
        let imp = ops.get(id).expect("listed id resolves");
        let params = imp.default_params();
        imp.validate_params(&params).unwrap_or_else(|e| {
            panic!("{id}: default params validate: {} {:?}", e.message, e.code)
        });
        let mut out = [[0.0f32; 4]; 8];
        imp.apply_cpu(&params, rect, &input, &mut out)
            .unwrap_or_else(|e| panic!("{id}: apply_cpu on defaults: {} {:?}", e.message, e.code));
        for (i, (o, inp)) in out.iter().zip(&input).enumerate() {
            assert!(
                o[3].to_bits() == inp[3].to_bits(),
                "{id}: pixel {i} alpha changed {} -> {} — an adjustment wrote COVERAGE; \
                 the chain-edit dirty region math (note_op_composite_dirty, masked arm) \
                 leans on alpha preservation. Extend the region math BEFORE shipping \
                 this op (R1 ruling arm 3).",
                inp[3],
                o[3]
            );
        }
    }
}
