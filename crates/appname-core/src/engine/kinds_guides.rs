// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: contracts `guide.add` / `guide.move` / `guide.delete` /
//! `guide.clear` / `grid.set`; ES §24.2 (Guides doc-model, sorted h/v
//! vectors addressed by (orientation, index)), §10.3 guide family (no
//! target block; T topic:"guides" — headless receipts carry no topics,
//! standing divergence). CP-0.5 wave-4 kind tail.
//!
//! All five kinds record ONE op shape — a full-state §24.2 Guides value
//! delta — which realizes each contract's inverse (param inverses and
//! guide.clear's restore-lists delta are the same block both ways).
//! `guide.move` is class M; coalescing awaits the §10.5 machinery
//! (standing divergence). Grid persistence rides inside the §7.3 guides
//! object (schema-gap flagged; the $def permits additional properties).

use std::sync::Arc;

use serde_json::Value;

use crate::engine::error::EngineError;
use crate::engine::kind_impl::{KindContext, KindImpl, KindOutcome};
use crate::graph::document::{GridSettings, Guides};

pub struct GuideAddKind;
pub struct GuideMoveKind;
pub struct GuideDeleteKind;
pub struct GuideClearKind;
pub struct GridSetKind;

pub(crate) fn impls() -> Vec<Arc<dyn KindImpl>> {
    vec![
        Arc::new(GuideAddKind),
        Arc::new(GuideMoveKind),
        Arc::new(GuideDeleteKind),
        Arc::new(GuideClearKind),
        Arc::new(GridSetKind),
    ]
}

fn orientation(cx: &KindContext<'_>) -> Result<bool, EngineError> {
    match cx.params().get("orientation").and_then(Value::as_str) {
        Some("horizontal") => Ok(true),
        Some("vertical") => Ok(false),
        Some(other) => Err(EngineError::range(
            "orientation",
            format!("unknown \"{other}\""),
        )),
        None => Err(EngineError::missing("orientation")),
    }
}

fn position(cx: &KindContext<'_>) -> Result<f64, EngineError> {
    let v = cx
        .params()
        .get("position")
        .and_then(Value::as_f64)
        .ok_or_else(|| EngineError::missing("position"))?;
    // Non-finite policy: NaN/±Inf never enter the sorted vectors.
    if !v.is_finite() {
        return Err(EngineError::range("position", "position must be finite"));
    }
    Ok(v)
}

fn index(cx: &KindContext<'_>, len: usize) -> Result<usize, EngineError> {
    let i = cx
        .params()
        .get("index")
        .and_then(Value::as_u64)
        .ok_or_else(|| EngineError::missing("index"))? as usize;
    if i >= len {
        return Err(EngineError::range(
            "index",
            format!("index {i} beyond the sorted vector (len {len}) — live-state check"),
        ));
    }
    Ok(i)
}

fn sort_vec(v: &mut [f64]) {
    v.sort_by(|a, b| a.partial_cmp(b).expect("finite positions"));
}

impl KindImpl for GuideAddKind {
    fn kind(&self) -> &'static str {
        "guide.add"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let horizontal = orientation(cx)?;
        let pos = position(cx)?; // off-canvas positions are legal (§24.2)
        let mut g = cx.document().guides().clone();
        let vec = if horizontal { &mut g.h } else { &mut g.v };
        vec.push(pos);
        sort_vec(vec);
        cx.set_guides(g)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for GuideMoveKind {
    fn kind(&self) -> &'static str {
        "guide.move"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let horizontal = orientation(cx)?;
        let pos = position(cx)?;
        let mut g = cx.document().guides().clone();
        let vec = if horizontal { &mut g.h } else { &mut g.v };
        let i = index(cx, vec.len())?;
        vec[i] = pos;
        sort_vec(vec); // crossing another guide re-sorts (§24.2)
        cx.set_guides(g)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for GuideDeleteKind {
    fn kind(&self) -> &'static str {
        "guide.delete"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let horizontal = orientation(cx)?;
        let mut g = cx.document().guides().clone();
        let vec = if horizontal { &mut g.h } else { &mut g.v };
        let i = index(cx, vec.len())?;
        vec.remove(i);
        cx.set_guides(g)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for GuideClearKind {
    fn kind(&self) -> &'static str {
        "guide.clear"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let mut g = cx.document().guides().clone();
        g.h.clear();
        g.v.clear();
        // Empty set → §40.7 no-op push (set_guides short-circuits equals).
        cx.set_guides(g)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for GridSetKind {
    fn kind(&self) -> &'static str {
        "grid.set"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let spacing = cx
            .params()
            .get("spacing")
            .and_then(Value::as_f64)
            .ok_or_else(|| EngineError::missing("spacing"))?;
        if !(0.1..=10000.0).contains(&spacing) {
            return Err(EngineError::range("spacing", "0.1..=10000 document px"));
        }
        let subdivisions = cx
            .params()
            .get("subdivisions")
            .and_then(Value::as_u64)
            .ok_or_else(|| EngineError::missing("subdivisions"))?;
        if !(1..=64).contains(&subdivisions) {
            return Err(EngineError::range("subdivisions", "1..=64"));
        }
        let show = cx
            .params()
            .get("show")
            .and_then(Value::as_bool)
            .ok_or_else(|| EngineError::missing("show"))?;
        let color = match cx.params().get("color") {
            None | Some(Value::Null) => None,
            Some(c) => {
                let obj = c
                    .as_object()
                    .ok_or_else(|| EngineError::range("color", "§22.1 object"))?;
                let comps = obj.get("comps").and_then(Value::as_array);
                if obj.get("model").and_then(Value::as_str).is_none()
                    || comps.map(|a| a.len()) != Some(4)
                {
                    return Err(EngineError::range(
                        "color",
                        "§22.1 color: model + 4 comps required",
                    ));
                }
                Some(c.clone())
            }
        };
        let mut g = cx.document().guides().clone();
        g.grid = Some(GridSettings {
            spacing,
            subdivisions: subdivisions as u32,
            show,
            color,
        });
        cx.set_guides(g)?;
        Ok(KindOutcome::default())
    }
}

/// The persisted-empty default (loader parity; doc.new leaves it default).
pub fn default_guides() -> Guides {
    Guides::default()
}

#[cfg(test)]
mod tests {

    use crate::engine::error::ErrorCode;
    use crate::engine::kind_impl::EngineAssembly;
    use crate::engine::Engine;
    use serde_json::json;

    fn ready() -> Engine {
        let mut e = crate::engine::kinds::register(EngineAssembly::new()).build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "60１d0000-0000-4000-8000-000000000001".replace('１', "1"),
                "kind": "doc.new",
                "params": {"width": 64, "height": 64},
            })
            .to_string(),
        )
        .unwrap();
        e
    }

    fn env(n: u32, kind: &str, params: serde_json::Value) -> String {
        json!({
            "schema": "appname.command.v1",
            "id": format!("601d0000-0000-4000-8000-0001{n:08x}"),
            "kind": kind,
            "params": params,
        })
        .to_string()
    }

    #[test]
    fn guide_family_round_trips_sorted_and_typed() {
        let mut e = ready();
        let before = e.state_hash();
        e.apply_json(&env(
            1,
            "guide.add",
            json!({"orientation": "vertical", "position": 960}),
        ))
        .unwrap();
        e.apply_json(&env(
            2,
            "guide.add",
            json!({"orientation": "vertical", "position": 100}),
        ))
        .unwrap();
        e.apply_json(&env(
            3,
            "guide.add",
            json!({"orientation": "horizontal", "position": -5.5}),
        ))
        .unwrap();
        let g = e.document().unwrap().guides().clone();
        assert_eq!(g.v, vec![100.0, 960.0], "sorted vectors (§24.2)");
        assert_eq!(g.h, vec![-5.5], "off-canvas legal");
        assert_ne!(e.state_hash(), before, "guides fold when present");
        // move index 0 across index 1: re-sorts.
        e.apply_json(&env(
            4,
            "guide.move",
            json!({"orientation": "vertical", "index": 0, "position": 1000}),
        ))
        .unwrap();
        assert_eq!(e.document().unwrap().guides().v, vec![960.0, 1000.0]);
        e.undo().unwrap();
        assert_eq!(
            e.document().unwrap().guides().v,
            vec![100.0, 960.0],
            "param inverse"
        );
        e.redo().unwrap();
        // delete + clear + live-state index checks.
        e.apply_json(&env(
            5,
            "guide.delete",
            json!({"orientation": "vertical", "index": 1}),
        ))
        .unwrap();
        assert_eq!(e.document().unwrap().guides().v, vec![960.0]);
        let err = e
            .apply_json(&env(
                6,
                "guide.move",
                json!({"orientation": "vertical", "index": 5, "position": 1}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::OutOfRange,
            "index beyond vector (live-state)"
        );
        e.apply_json(&env(7, "guide.clear", json!({}))).unwrap();
        assert!(e.document().unwrap().guides().h.is_empty());
        assert!(e.document().unwrap().guides().v.is_empty());
        // clear restores BOTH lists on undo (delta inverse).
        e.undo().unwrap();
        assert_eq!(e.document().unwrap().guides().v, vec![960.0]);
        assert_eq!(e.document().unwrap().guides().h, vec![-5.5]);
        e.redo().unwrap();
        // empty-set clear = no-op push.
        let l = e.journal().len();
        e.apply_json(&env(8, "guide.clear", json!({}))).unwrap();
        assert_eq!(e.journal().len(), l + 1);
        // non-finite position refuses.
        let err = e
            .apply_json(&env(
                9,
                "guide.add",
                json!({"orientation": "vertical", "position": "NaN"}),
            ))
            .unwrap_err();
        // Engine taxonomy: mistyped params emit out_of_range at stage 5
        // (the contracts' stage-3 missing_field spelling is a standing
        // validator-taxonomy divergence, lane-log fold-in note).
        assert_eq!(err.code, ErrorCode::OutOfRange);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn grid_set_full_state_round_trips_and_persists() {
        let mut e = ready();
        e.apply_json(&env(
            20,
            "grid.set",
            json!({"spacing": 64, "subdivisions": 4, "show": true,
                    "color": {"model": "rgb", "comps": [0.5, 0.5, 0.5, 0]}}),
        ))
        .unwrap();
        let g = e.document().unwrap().guides().grid.clone().unwrap();
        assert_eq!((g.spacing, g.subdivisions, g.show), (64.0, 4, true));
        let with_grid = e.state_hash();
        // Full-state both directions: previous block re-applies on undo.
        e.apply_json(&env(
            21,
            "grid.set",
            json!({"spacing": 32, "subdivisions": 2, "show": false}),
        ))
        .unwrap();
        e.undo().unwrap();
        assert_eq!(e.state_hash(), with_grid);
        e.redo().unwrap();
        // Range rejects.
        let err = e
            .apply_json(&env(
                22,
                "grid.set",
                json!({"spacing": 0.05, "subdivisions": 4, "show": true}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        let err = e
            .apply_json(&env(
                23,
                "grid.set",
                json!({"spacing": 64, "subdivisions": 4}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::MissingField,
            "show is required (full-state)"
        );
        // guide.add + grid persist through the package (guides object).
        e.apply_json(&env(
            24,
            "guide.add",
            json!({"orientation": "vertical", "position": 8}),
        ))
        .unwrap();
        let bytes = crate::io::package::save_package_bytes(&e).unwrap();
        let loaded = crate::io::package::load_package_bytes_with(
            &crate::engine::kinds::register(EngineAssembly::new()),
            &bytes,
        )
        .unwrap();
        assert_eq!(
            loaded.engine.document().unwrap().guides(),
            e.document().unwrap().guides()
        );
        assert_eq!(
            loaded.engine.state_hash_persistent(),
            e.state_hash_persistent()
        );
    }

    #[test]
    fn doc_duplicate_dagger_door() {
        let mut e = ready();
        e.apply_json(&env(
            30,
            "guide.add",
            json!({"orientation": "vertical", "position": 12}),
        ))
        .unwrap();
        let journal_len = e.journal().len();
        let dup = e.duplicate().unwrap();
        // † semantics: no journal record on the source; the duplicate
        // carries the journal provenance with an empty in-process history
        // (its own base — loaded-parts posture, §10.6).
        assert_eq!(e.journal().len(), journal_len, "source journal untouched");
        assert_eq!(dup.journal().len(), journal_len);
        assert!(
            !dup.undoable_in_process(),
            "duplicate opens with empty history"
        );
        assert_eq!(dup.state_hash(), e.state_hash(), "COW deep copy");
        // The apply-path kind stays typed by design (handle surface).
        let err = e
            .apply_json(
                &json!({
                    "schema": "appname.command.v1",
                    "id": "601d0000-0000-4000-8000-000000000099",
                    "kind": "doc.duplicate",
                })
                .to_string(),
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
    }
}
