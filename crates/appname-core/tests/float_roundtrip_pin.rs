// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §10 / D-48 determinism spine — byte-exact replay requires
//! parse(write(v)) == v for every float that rides persisted JSON.
//!
//! FEATURE-PRESENCE PIN (retroactive float_roundtrip review, finding R1):
//! serde_json's `float_roundtrip` feature is what makes its float parse
//! correctly rounded; without it the parser is best-effort and can land
//! 1 ulp off. Cargo feature unification is PER-BUILD-GRAPH, not
//! per-workspace — so this pin lives in appname-core's OWN suite: it runs
//! (and reds on feature loss) in the single-package graph
//! `cargo test -p appname-core`, which no other crate's declaration can
//! protect. The control value is the review's kappa-arc constant,
//! 88 + 8*kappa (kappa = 4/3*(sqrt(2)-1), the rounded_rect arc control
//! from the original O-TEXTVEC DEFECT-1): serde_json's default parser
//! picks the wrong neighbor for it (decimal distance 1.01e-14 vs the
//! correctly-rounded 4.15e-15), so this single literal separates the two
//! parsers deterministically. Election record: deps/serde.yaml.

/// The kappa-arc literal as it appears in persisted JSON.
const KAPPA_ARC_LITERAL: &str = "92.41827799864635";

/// IEEE-754 bits of the correctly-rounded f64 neighbor of the literal.
/// (The default best-effort parser yields 0x40571AC5111534A3 — 1 ulp up.)
const KAPPA_ARC_BITS: u64 = 0x4057_1AC5_1115_34A2;

/// Direct typed parse: the path envelope param deserialization takes.
#[test]
fn float_roundtrip_feature_present_typed_parse() {
    let v: f64 =
        serde_json::from_str(KAPPA_ARC_LITERAL).expect("kappa-arc literal must parse as f64");
    assert_eq!(
        v.to_bits(),
        KAPPA_ARC_BITS,
        "serde_json parsed {KAPPA_ARC_LITERAL} to bits {:#018X}, expected \
         {KAPPA_ARC_BITS:#018X} (the correctly-rounded neighbor). The \
         float_roundtrip feature is MISSING from this build graph — declare \
         it on appname-core's serde_json dependency (review R1; \
         deps/serde.yaml).",
        v.to_bits()
    );
}

/// Value-tree parse: the path `serde_json::Value` envelope handling takes.
#[test]
fn float_roundtrip_feature_present_value_parse() {
    let v: serde_json::Value = serde_json::from_str(KAPPA_ARC_LITERAL)
        .expect("kappa-arc literal must parse as a JSON value");
    let f = v.as_f64().expect("number");
    assert_eq!(f.to_bits(), KAPPA_ARC_BITS);
}

/// The invariant the feature exists to uphold: parse(write(v)) == v,
/// bit-exact, for the control value.
#[test]
fn float_roundtrip_parse_write_identity() {
    let v = f64::from_bits(KAPPA_ARC_BITS);
    let written = serde_json::to_string(&v).expect("write");
    let reparsed: f64 = serde_json::from_str(&written).expect("reparse");
    assert_eq!(reparsed.to_bits(), v.to_bits());
}
