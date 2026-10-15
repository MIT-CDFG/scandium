// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The §16.3 CPU-reference corpus, slice 1 (plan §2.4). Each module carries
//! its filter's full normative block citation, exact param schema, and
//! oracle tests at the lane's kernel bar. Rights basis per module docs —
//! everything in slice 1 is era/standards/textbook-basis (dossier rows
//! cited in place; V39 discipline: select-and-screen judgments, no release
//! claims). Nothing from the walled list is referenced anywhere in this
//! tree.

pub mod box_blur;
pub mod custom;
pub mod gaussian;
pub mod liquify;
pub mod median;
pub mod noise;
pub mod shared;
pub mod sharpen;

use std::sync::Arc;

use crate::registry::FilterRegistry;

/// The slice-1 registry assembly (grows per module; the command seam
/// consumes exactly this when O-CORE's OpNode work publishes). CP-1.0
/// warps row: `filter.liquify` joins as the §45.4 checkpoint-arm
/// instance (the §10.3 whole-canvas class the slice-1 note reserved).
pub fn slice1_registry() -> FilterRegistry {
    FilterRegistry::new()
        .with(Arc::new(box_blur::BoxBlur))
        .with(Arc::new(gaussian::GaussianBlur))
        .with(Arc::new(sharpen::UnsharpMask))
        .with(Arc::new(sharpen::HighPass))
        .with(Arc::new(noise::AddNoise))
        .with(Arc::new(median::Median))
        .with(Arc::new(custom::CustomConvolve))
        .with(Arc::new(liquify::Liquify))
}

#[cfg(test)]
mod strictness_tests {
    use super::*;
    use serde_json::{json, Value};

    /// Minimal schema-valid params per registered filter (everything is
    /// optional-with-defaults except liquify's required field payload).
    fn minimal_valid(id: &str) -> Value {
        match id {
            "filter.liquify" => json!({
                "origin": [0, 0], "size": [2, 2], "disp": vec![0.0; 8],
            }),
            _ => json!({}),
        }
    }

    /// The 2026-08-17 strictness ruling, held SYSTEMATICALLY: every
    /// registered filter accepts its minimal-valid params and refuses
    /// an unknown key typed — no registrant can drift back into silent
    /// tolerance (the O-UI gaussian probe class), because the loop runs
    /// over the LIVE registry, not a hand-kept list.
    #[test]
    fn every_registered_filter_refuses_unknown_keys() {
        let reg = slice1_registry();
        assert!(!reg.is_empty());
        for f in reg.filters() {
            let ok = minimal_valid(f.id());
            assert!(
                f.validate_strict(&ok).is_ok(),
                "{}: minimal-valid params must pass",
                f.id()
            );
            let mut bogus = ok.clone();
            bogus["definitelyNotAParam"] = json!(1);
            let err = f.validate_strict(&bogus).unwrap_err();
            let text = format!("{err}");
            assert!(
                text.contains("definitelyNotAParam"),
                "{}: unknown key must reject typed, got: {text}",
                f.id()
            );
        }
    }

    /// Schema well-formedness self-check: every registered param_schema
    /// parses, declares the strict root (`additionalProperties: false`),
    /// and every `$ref` it carries resolves against the schema ROOT
    /// (the misplaced-$defs class fixed in the adjust corpus this
    /// slice cannot regrow here unnoticed).
    #[test]
    fn every_registered_schema_is_strict_and_wellformed() {
        fn collect_refs(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::Object(m) => {
                    if let Some(r) = m.get("$ref").and_then(Value::as_str) {
                        out.push(r.to_string());
                    }
                    for x in m.values() {
                        collect_refs(x, out);
                    }
                }
                Value::Array(a) => {
                    for x in a {
                        collect_refs(x, out);
                    }
                }
                _ => {}
            }
        }
        for f in slice1_registry().filters() {
            let schema: Value = serde_json::from_str(f.param_schema())
                .unwrap_or_else(|e| panic!("{}: schema unparsable: {e}", f.id()));
            assert_eq!(
                schema.get("additionalProperties"),
                Some(&Value::Bool(false)),
                "{}: root must declare additionalProperties:false",
                f.id()
            );
            let mut refs = Vec::new();
            collect_refs(&schema, &mut refs);
            for r in refs {
                let name = r
                    .strip_prefix("#/$defs/")
                    .unwrap_or_else(|| panic!("{}: unsupported $ref form {r}", f.id()));
                assert!(
                    schema.get("$defs").and_then(|d| d.get(name)).is_some(),
                    "{}: $ref {r} must resolve at the schema root",
                    f.id()
                );
            }
        }
    }

    /// Wrong-TYPE on a known key is the same silent-no-op class and
    /// refuses (the noise `monochromatic: "yes"` case that previously
    /// defaulted to false with ok=true).
    #[test]
    fn typoed_types_on_known_keys_refuse() {
        let reg = slice1_registry();
        let noise = reg.get("filter.add_noise").unwrap();
        assert!(noise
            .validate_strict(&json!({"monochromatic": "yes"}))
            .is_err());
        let gaussian = reg.get("filter.gaussian_blur").unwrap();
        assert!(gaussian.validate_strict(&json!({"radius": "big"})).is_err());
        // …while the probe's range case stays its own typed reject.
        assert!(gaussian.validate_strict(&json!({"radius": 2000})).is_err());
        assert!(gaussian.validate_strict(&json!({"radius": 8.0})).is_ok());
    }
}
