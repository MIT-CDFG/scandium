// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! Polled event queue: `appname.event.dirtyRects.v1` +
//! `appname.event.history.v1` payloads.
//!
//! Spec: ES §9.5 (event payload schemas — `schema`, `docId`, `thread`
//! fields; per-command NORMATIVE relative order `treeChanged` →
//! `dirtyRects` → `history`; hosts ignore unknown fields AND unknown
//! schemas per §2.8). v1 scope per the plan §2.2 row was dirty rects +
//! history/cursor events; `treeChanged` joined at the ABI-growth round —
//! its documented trigger ("when the layer-panel surface grows") was
//! pulled by O-UI's chrome wave (layers panel v2 tree/masks/blend truth,
//! reports/O-UI-log.md ABI-GROWTH REQUEST item 3). v1 executes commands
//! synchronously on the caller's thread, which is the command thread —
//! every payload declares `"thread": "command"`.
//!
//! `treeChanged` derivation (§9.5 shape `{schema, docId, thread, kind,
//! layerIds}` with `kind` ∈ added|removed|moved|props): the queue compares
//! a before/after structural snapshot of the layer tree — membership for
//! added/removed, (parent, index) for moved, and a props fingerprint
//! (name, blend, opacity, fillOpacity, visible, locks, clipToBelow,
//! colorLabel, isolated, transform, mask list + per-mask props) for props
//! — so the events are STATE truth, independent of which command produced
//! the change, and undo/redo emit the inverse of what their step emitted.
//! One event per kind per command (§9.5: a command that both moves and
//! re-labels emits two events, `moved` then `props`), emitted in the fixed
//! order added → removed → moved → props, all before `dirtyRects`.
//!
//! Delivery: [`crate::doc::appname_doc_poll_events`] drains the queue in
//! emission order. A dirtyRects payload carries ONE OR MORE rects — at
//! most 8: receipt rects pass through the §3.7 coalescer
//! ([`appname_render::RegionSet`], the same normative algorithm the
//! renderer uses — ES §3.7: rects within the 8 px `COALESCE_SLOP` merge by
//! inflate-intersect to fixpoint; a set exceeding 8 rects collapses to its
//! single bounding box), so the payload's union always covers every
//! receipt rect. Consumers must iterate the whole `rects` array, never
//! assume `rects[0]` covers the change. The queue is bounded: past
//! [`MAX_QUEUED_EVENTS`] the oldest events coalesce (dirty rects union
//! into one covering rect; only the newest history row survives) —
//! refresh semantics lose nothing, since a union repaint covers its parts
//! and history state is a snapshot.

use appname_core::{DirtyRect, Engine, IRect};
use serde_json::{json, Value};

use crate::registry::DocState;

/// Queue bound before coalescing kicks in (documented in the header).
pub(crate) const MAX_QUEUED_EVENTS: usize = 4096;

/// One layer's structural row in a tree snapshot: placement + a props
/// fingerprint (JSON text of everything the layer panel displays — cheap
/// to compare, never exposed on the wire).
#[derive(PartialEq)]
struct TreeRow {
    parent: Option<String>,
    index: usize,
    props: String,
}

/// Document identity + canvas extent + layer-tree structure captured around
/// a mutation (either side may be absent: the pre-init shell has neither).
pub(crate) struct DocSnapshot {
    pub doc_id: Option<String>,
    pub canvas: Option<IRect>,
    /// layerId (uuid string) -> structural row, in z-order walk order
    /// (order kept for deterministic event payloads).
    tree: Vec<(String, TreeRow)>,
}

fn tree_rows(engine: &Engine) -> Vec<(String, TreeRow)> {
    let Some(d) = engine.document() else {
        return Vec::new();
    };
    d.z_order_walk()
        .into_iter()
        .filter_map(|id| {
            let n = d.layer(id)?;
            let parent = d.parent_of(id).map(|p| p.as_uuid().to_string());
            let index = parent
                .as_ref()
                .and_then(|_| d.parent_of(id))
                .and_then(|p| d.children_of(p))
                .and_then(|c| c.iter().position(|x| *x == id))
                .unwrap_or(0);
            // Props fingerprint: the §2.6 shared block + the mask surface —
            // exactly what the layer panel renders (module doc).
            let masks: Vec<Value> = n
                .masks()
                .iter()
                .filter_map(|m| d.mask(*m))
                .map(|m| {
                    json!({
                        "id": m.id().as_uuid().to_string(),
                        "enabled": m.enabled(),
                        "inverted": m.inverted(),
                        "density": m.density(),
                        "feather": m.feather(),
                        "linked": m.linked(),
                    })
                })
                .collect();
            let props = json!({
                "name": n.name(),
                "blend": n.blend(),
                "opacity": n.opacity(),
                "fillOpacity": n.fill_opacity(),
                "visible": n.visible(),
                "locks": n.locks(),
                "clipToBelow": n.clip_to_below(),
                "colorLabel": n.color_label(),
                "isolated": n.isolated(),
                "transform": format!("{:?}", n.transform()),
                "masks": masks,
            })
            .to_string();
            Some((
                id.as_uuid().to_string(),
                TreeRow {
                    parent,
                    index,
                    props,
                },
            ))
        })
        .collect()
}

pub(crate) fn snapshot(engine: &Engine) -> DocSnapshot {
    match engine.document() {
        Some(d) => DocSnapshot {
            doc_id: Some(d.id().as_uuid().to_string()),
            canvas: Some(d.geometry().canvas_rect()),
            tree: tree_rows(engine),
        },
        None => DocSnapshot {
            doc_id: None,
            canvas: None,
            tree: Vec::new(),
        },
    }
}

/// Option-lifted `IRect::union` (which is itself empty-aware, ES §2.1).
fn union(a: Option<IRect>, b: Option<IRect>) -> Option<IRect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(&b)),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

fn rect_json(r: IRect) -> Value {
    json!([r.min.x, r.min.y, r.width(), r.height()])
}

/// Emit the §9.5 pair for one successful mutation (apply/undo/redo), in the
/// normative relative order: `dirtyRects` then `history`.
///
/// * Dirty rects come from the RECEIPT (wave-2b: core receipts carry
///   per-layer `DirtyRect`s — tile-writing kinds report each touched
///   tile's pixel rect, structural kinds node bounds before∪after; undo and
///   redo rows echo the SAME rects), §3.7-coalesced for the payload
///   (O-WATCH #2 MINOR-1): ≤ 8 composite-space `[x, y, w, h]` rects whose
///   union covers every receipt rect (layer attribution stays
///   receipt-side, §9.5). Empty rects are ignored and an all-empty list
///   emits NO dirtyRects event (e.g. `doc.new` with a transparent
///   background changes no visible pixel).
/// * `docId` = the document after the mutation, falling back to the one
///   before (undo of `doc.new` returns to the pre-init shell).
/// * history depths are journal truth: `undoDepth` = cursor, `redoDepth` =
///   journal length - cursor (§10.3 linear history).
pub(crate) fn emit_mutation(
    state: &mut DocState,
    before: &DocSnapshot,
    label_kind: &str,
    dirty: &[DirtyRect],
) {
    let after = snapshot(&state.engine);
    let doc_id = after.doc_id.clone().or_else(|| before.doc_id.clone());
    let Some(doc_id) = doc_id else {
        // Unreachable: a successful mutation has a document on at least one
        // side. Skip rather than emit a schema-invalid payload.
        return;
    };
    // §9.5 treeChanged FIRST (normative order treeChanged → dirtyRects →
    // history): structural diff of the before/after snapshots (module doc).
    emit_tree_changed(state, &doc_id, &before.tree, &after.tree);
    // §3.7 coalescing (O-WATCH #2 MINOR-1): receipt rects pass through the
    // ONE normative coalescer (appname-render's `RegionSet` — 8 px
    // inflate-intersect merge to fixpoint; a set exceeding 8 rects
    // collapses to its bounding box), so the payload carries at most
    // REGION_SET_CAP rects whose union covers every receipt rect.
    let mut set = appname_render::RegionSet::new();
    for d in dirty {
        set.add(d.rect); // empty rects are ignored by the §3.7 algorithm
    }
    if !set.is_empty() {
        let rects: Vec<Value> = set.rects().iter().map(|r| rect_json(*r)).collect();
        push(
            state,
            json!({
                "schema": "appname.event.dirtyRects.v1",
                "docId": doc_id,
                "thread": "command",
                "scope": "composite",
                "rects": rects,
            }),
        );
    }
    let cursor = state.engine.journal_cursor();
    let len = state.engine.journal().len();
    push(
        state,
        json!({
            "schema": "appname.event.history.v1",
            "docId": doc_id,
            "thread": "command",
            "undoDepth": cursor,
            "redoDepth": len - cursor,
            "labelMessageId": label_kind,
            "coalesced": false,
        }),
    );
}

/// Diff two tree snapshots and queue up to four `treeChanged` events
/// (added / removed / moved / props — §9.5 shape and one-event-per-kind
/// rule; module doc). `layerIds` keep snapshot (z-order walk) order.
fn emit_tree_changed(
    state: &mut DocState,
    doc_id: &str,
    before: &[(String, TreeRow)],
    after: &[(String, TreeRow)],
) {
    let b: std::collections::HashMap<&str, &TreeRow> =
        before.iter().map(|(id, r)| (id.as_str(), r)).collect();
    let a: std::collections::HashMap<&str, &TreeRow> =
        after.iter().map(|(id, r)| (id.as_str(), r)).collect();

    let added: Vec<&str> = after
        .iter()
        .filter(|(id, _)| !b.contains_key(id.as_str()))
        .map(|(id, _)| id.as_str())
        .collect();
    let removed: Vec<&str> = before
        .iter()
        .filter(|(id, _)| !a.contains_key(id.as_str()))
        .map(|(id, _)| id.as_str())
        .collect();
    let moved: Vec<&str> = after
        .iter()
        .filter(|(id, r)| {
            b.get(id.as_str())
                .is_some_and(|old| (old.parent != r.parent) || (old.index != r.index))
        })
        .map(|(id, _)| id.as_str())
        .collect();
    let props: Vec<&str> = after
        .iter()
        .filter(|(id, r)| b.get(id.as_str()).is_some_and(|old| old.props != r.props))
        .map(|(id, _)| id.as_str())
        .collect();

    for (kind, ids) in [
        ("added", added),
        ("removed", removed),
        ("moved", moved),
        ("props", props),
    ] {
        if ids.is_empty() {
            continue;
        }
        push(
            state,
            json!({
                "schema": "appname.event.treeChanged.v1",
                "docId": doc_id,
                "thread": "command",
                "kind": kind,
                "layerIds": ids,
            }),
        );
    }
}

/// Seed event after `appname_doc_open`: the whole canvas is fresh (the host
/// has rendered nothing yet). No history event — opening changes no stack.
pub(crate) fn emit_open_seed(state: &mut DocState) {
    let snap = snapshot(&state.engine);
    if let (Some(doc_id), Some(canvas)) = (snap.doc_id, snap.canvas) {
        push(
            state,
            json!({
                "schema": "appname.event.dirtyRects.v1",
                "docId": doc_id,
                "thread": "command",
                "scope": "composite",
                "rects": [rect_json(canvas)],
            }),
        );
    }
}

fn push(state: &mut DocState, event: Value) {
    state.events.push(event);
    if state.events.len() > MAX_QUEUED_EVENTS {
        coalesce(state);
    }
}

/// Bound the queue: union every queued dirty rect (per docId irrelevant in
/// v1 — one document per handle) into one covering dirtyRects event, union
/// queued treeChanged `layerIds` per kind (coverage never shrinks — a host
/// at overflow refreshes the listed ids against info truth), keep only the
/// newest history event, preserve the tree-before-dirty-before-history
/// order.
fn coalesce(state: &mut DocState) {
    let mut dirty_union: Option<IRect> = None;
    let mut dirty_template: Option<Value> = None;
    let mut last_history: Option<Value> = None;
    // Per-kind ordered id unions for treeChanged (§9.5 kinds).
    let mut tree: Vec<(String, Vec<String>)> = Vec::new();
    let mut tree_template: Option<Value> = None;
    for ev in state.events.drain(..) {
        match ev.get("schema").and_then(Value::as_str) {
            Some("appname.event.treeChanged.v1") => {
                let kind = ev
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("props")
                    .to_string();
                let ids: Vec<String> = ev
                    .get("layerIds")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let slot = match tree.iter_mut().find(|(k, _)| *k == kind) {
                    Some((_, v)) => v,
                    None => {
                        tree.push((kind, Vec::new()));
                        &mut tree.last_mut().expect("just pushed").1
                    }
                };
                for id in ids {
                    if !slot.contains(&id) {
                        slot.push(id);
                    }
                }
                tree_template = Some(ev);
            }
            Some("appname.event.dirtyRects.v1") => {
                if let Some(rects) = ev.get("rects").and_then(Value::as_array) {
                    for r in rects {
                        let (Some(x), Some(y), Some(w), Some(h)) = (
                            r.get(0).and_then(Value::as_i64),
                            r.get(1).and_then(Value::as_i64),
                            r.get(2).and_then(Value::as_i64),
                            r.get(3).and_then(Value::as_i64),
                        ) else {
                            continue;
                        };
                        // Payload rects are [x, y, w, h]; IRect is min/max.
                        dirty_union = union(
                            dirty_union,
                            Some(IRect::new(
                                x as i32,
                                y as i32,
                                x as i32 + w as i32,
                                y as i32 + h as i32,
                            )),
                        );
                    }
                }
                dirty_template = Some(ev);
            }
            Some("appname.event.history.v1") => last_history = Some(ev),
            _ => {}
        }
    }
    if let Some(template) = tree_template {
        // §9.5 kind order: added → removed → moved → props (module doc).
        for kind in ["added", "removed", "moved", "props"] {
            if let Some((_, ids)) = tree.iter().find(|(k, _)| k == kind) {
                if !ids.is_empty() {
                    let mut ev = template.clone();
                    ev["kind"] = json!(kind);
                    ev["layerIds"] = json!(ids);
                    state.events.push(ev);
                }
            }
        }
    }
    if let (Some(mut template), Some(rect)) = (dirty_template, dirty_union) {
        template["rects"] = json!([rect_json(rect)]);
        state.events.push(template);
    }
    if let Some(h) = last_history {
        state.events.push(h);
    }
}

/// Drain the queue into one JSON array (emission order preserved).
pub(crate) fn drain_json(state: &mut DocState) -> String {
    let events: Vec<Value> = state.events.drain(..).collect();
    Value::Array(events).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_state_with(background: &str) -> (DocState, Vec<DirtyRect>) {
        let mut engine = Engine::new();
        let receipt = engine
            .apply_json(
                &json!({
                    "schema": "appname.command.v1",
                    "id": "c0000000-0000-4000-8000-000000000001",
                    "kind": "doc.new",
                    "params": {"width": 32, "height": 16, "background": background}
                })
                .to_string(),
            )
            .unwrap();
        (DocState::new(engine), receipt.dirty)
    }

    fn doc_state() -> (DocState, Vec<DirtyRect>) {
        doc_state_with("white")
    }

    #[test]
    fn mutation_emits_dirty_then_history_with_spec_shapes() {
        let (mut state, dirty) = doc_state();
        let before = DocSnapshot {
            doc_id: None,
            canvas: None,
            tree: Vec::new(),
        };
        emit_mutation(&mut state, &before, "doc.new", &dirty);
        // §9.5 normative order: treeChanged → dirtyRects → history.
        assert_eq!(state.events.len(), 3);
        let tree = &state.events[0];
        assert_eq!(tree["schema"], "appname.event.treeChanged.v1");
        assert_eq!(tree["thread"], "command");
        assert_eq!(tree["kind"], "added");
        // doc.new added the background layer (root is not a listed row).
        assert_eq!(tree["layerIds"].as_array().unwrap().len(), 1);
        let ev = &state.events[1];
        assert_eq!(ev["schema"], "appname.event.dirtyRects.v1");
        assert_eq!(ev["thread"], "command");
        assert_eq!(ev["scope"], "composite");
        // Receipt truth: doc.new (white) reports the background node bounds.
        assert_eq!(ev["rects"], json!([[0, 0, 32, 16]]));
        assert!(ev["docId"].is_string());
        let hist = &state.events[2];
        assert_eq!(hist["schema"], "appname.event.history.v1");
        assert_eq!(hist["undoDepth"], 1);
        assert_eq!(hist["redoDepth"], 0);
        assert_eq!(hist["labelMessageId"], "doc.new");
        assert_eq!(hist["coalesced"], false);
    }

    #[test]
    fn empty_receipt_dirty_emits_history_only() {
        // doc.new with a transparent background changes no visible pixel:
        // the receipt's dirty list is empty (core contract) and no
        // dirtyRects event is emitted — treeChanged (the layer WAS added)
        // and history still are.
        let (mut state, dirty) = doc_state_with("transparent");
        assert!(
            dirty.is_empty(),
            "transparent doc.new reports no dirty rects"
        );
        let before = DocSnapshot {
            doc_id: None,
            canvas: None,
            tree: Vec::new(),
        };
        emit_mutation(&mut state, &before, "doc.new", &dirty);
        assert_eq!(state.events.len(), 2);
        assert_eq!(state.events[0]["schema"], "appname.event.treeChanged.v1");
        assert_eq!(state.events[0]["kind"], "added");
        assert_eq!(state.events[1]["schema"], "appname.event.history.v1");
    }

    /// Build a dirty list on the doc's own layer ids (surface attribution
    /// is receipt-side only; the payload is composite-space).
    fn dirty_of(state: &DocState, rects: &[IRect]) -> Vec<DirtyRect> {
        let layer = state.engine.document().unwrap().z_order_walk()[0];
        rects
            .iter()
            .map(|r| DirtyRect { layer, rect: *r })
            .collect()
    }

    fn emitted_rects(state: &mut DocState, dirty: &[DirtyRect]) -> Value {
        let before = snapshot(&state.engine);
        state.events.clear();
        emit_mutation(state, &before, "doc.new", dirty);
        state.events[0]["rects"].clone()
    }

    #[test]
    fn payload_rects_follow_the_contract_coalescing() {
        // ES §3.7 "Coalescing algorithm. RegionSet::add(r), with
        // COALESCE_SLOP: i32 = 8 px: 1. Ignore empty rects. 2. Merge pass:
        // if r.inflate(COALESCE_SLOP, COALESCE_SLOP) intersects an existing
        // rect, remove that rect, set r = r.union(existing), and restart
        // the scan — repeat to fixpoint […]. 3. Insert r. If the set would
        // exceed 8 rects, collapse the entire set to its single bounding
        // box." — pinned here on the event payload path (O-WATCH #2
        // MINOR-1); the implementation is the renderer's own RegionSet.
        let (mut state, _) = doc_state();

        // Rule 1: empty rects are ignored; duplicates trivially merge.
        let dirty = dirty_of(
            &state,
            &[IRect::new(0, 0, 8, 8), IRect::EMPTY, IRect::new(0, 0, 8, 8)],
        );
        assert_eq!(emitted_rects(&mut state, &dirty), json!([[0, 0, 8, 8]]));

        // Rule 2: rects within the 8 px slop union (adjacent tile rects
        // chain to one); rects farther than the slop stay separate.
        let dirty = dirty_of(
            &state,
            &[
                IRect::new(0, 0, 8, 8),
                IRect::new(15, 15, 23, 23),     // gap 7 < slop: merges
                IRect::new(100, 100, 108, 108), // far: stays
            ],
        );
        assert_eq!(
            emitted_rects(&mut state, &dirty),
            json!([[0, 0, 23, 23], [100, 100, 8, 8]])
        );
        // Half-open boundary: a gap of exactly the slop does NOT merge
        // (inflate(8) reaches x=16; [16,..) intersected half-open is empty).
        let dirty = dirty_of(&state, &[IRect::new(0, 0, 8, 8), IRect::new(16, 0, 24, 8)]);
        assert_eq!(
            emitted_rects(&mut state, &dirty),
            json!([[0, 0, 8, 8], [16, 0, 8, 8]])
        );
        // Fixpoint restart: a bridge rect glues two previously-separate
        // rects into one union.
        let dirty = dirty_of(
            &state,
            &[
                IRect::new(0, 0, 8, 8),
                IRect::new(40, 0, 48, 8),
                IRect::new(12, 0, 36, 8), // touches both within the slop
            ],
        );
        assert_eq!(emitted_rects(&mut state, &dirty), json!([[0, 0, 48, 8]]));

        // Rule 3: a 9th distant rect collapses the set to one bounding box.
        let nine: Vec<IRect> = (0..9)
            .map(|i| IRect::new(i * 100, 0, i * 100 + 8, 8))
            .collect();
        let dirty = dirty_of(&state, &nine);
        assert_eq!(emitted_rects(&mut state, &dirty), json!([[0, 0, 808, 8]]));
    }

    #[test]
    fn overflow_coalesces_to_covering_rect_plus_newest_history() {
        let (mut state, dirty) = doc_state();
        let before = snapshot(&state.engine);
        for _ in 0..(MAX_QUEUED_EVENTS / 2 + 1) {
            emit_mutation(&mut state, &before, "doc.new", &dirty);
        }
        assert!(state.events.len() <= MAX_QUEUED_EVENTS, "bounded");
        let drained = drain_json(&mut state);
        let arr: Vec<Value> = serde_json::from_str(&drained).unwrap();
        let dirty_count = arr
            .iter()
            .filter(|e| e["schema"] == "appname.event.dirtyRects.v1")
            .count();
        assert!(dirty_count >= 1, "the covering dirty event survives");
        assert!(state.events.is_empty(), "drain empties the queue");
    }

    #[test]
    fn drain_yields_empty_array_when_no_events() {
        let (mut state, _) = doc_state();
        assert_eq!(drain_json(&mut state), "[]");
    }
}
