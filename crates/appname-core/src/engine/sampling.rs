// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §45.2 (clone/heal SAMPLE SCOPE tri-state), §13.3 (magic wand /
//! quick-select `sampleAllLayers`), §4.4 (source-over), §4.6/§4.7
//! (effective visibility, pass-through groups), §11.2 (effective mask
//! coverage), §12.4 (flatten semantics). Contracts `clone.stroke`,
//! `heal.stroke`, `selection.set`.
//!
//! **The SAMPLING composite** — the read-only surface a command samples
//! FROM when its params ask for more than one layer. Three wire spellings
//! land here:
//!
//! | caller | wire | scope |
//! |---|---|---|
//! | `clone`/`heal` | `source:"composite"` + `sampleAllLayers:true` | [`SampleScope::Document`] |
//! | `clone`/`heal` | `source:{layerId}` + `sampleAllLayers:true` | [`SampleScope::AtOrBelow`] |
//! | `selection.set` `wand`/`quick` | `sampleAllLayers:true` | [`SampleScope::Document`] |
//!
//! # Which compositing truth this is (the load-bearing decision)
//!
//! The engine currently carries TWO compositing arithmetics: the LINEAR
//! reference export ([`crate::io::png_io::composite_plane_f32`] — decode
//! storage → linear premultiplied → source-over) and the STORAGE-SPACE
//! source-over that the stroke family and [`crate::engine::kinds_merge`]
//! (`layer.merge_down` / `layer.merge_visible` / `image.flatten`) share —
//! the standing interim divergence recorded in `kinds_merge`'s module doc.
//!
//! This door is the **storage-space** one, deliberately, because both
//! normative sentences describe a FLATTEN, not an export: §13.3 says the
//! wand samples "the flattened composite" and §45.2's tri-state table says
//! "the full document composite" / "the composite of the armed layer and
//! everything below it". Two invariants force the choice, and both are
//! pinned as tests:
//!
//! 1. **`sampleAllLayers` must not fork on a one-layer document.** The
//!    composite of a single layer IS that layer, so the `true` and `false`
//!    arms of the SAME tri-state must agree bit-for-bit. Storage-space
//!    source-over over a transparent accumulator returns the source
//!    verbatim (`scale = a/a = 1`, `ia = 1 − a` against zeros); the linear
//!    arm would round-trip u8 storage through `srgb_decode`/`srgb_encode`
//!    and unpremultiply, and would NOT. (Exact reach: alpha ≥
//!    `f32::EPSILON`, the merge math's standing `src[3].max(EPSILON)`
//!    guard — unreachable in u8/u16 storage, pinned as a test.)
//! 2. **Sampling the composite must agree with flattening first.**
//!    `image.flatten` then sample-one-layer, versus sample-all-layers on
//!    the unflattened document, are the same user-visible operation.
//!    `merge_onto` and this module now share ONE per-pixel routine
//!    ([`source_over_storage`]), so the arithmetic cannot drift.
//!
//!    **The exact reach of that agreement, stated rather than assumed.**
//!    Sharing `source_over_storage` pins the source-over STEP; it does
//!    not by itself pin the whole flatten. FOUR places where the two
//!    walks still differ, each stated so no reader has to infer it:
//!
//!    a. **Bottom-layer presentation props.** `merge_onto` seeds its
//!       accumulator with the RECEIVER's raw pixels and `image.flatten`
//!       then resets the receiver's presentation props to `1.0` (§12.4
//!       background-ize), so flatten DISCARDS the bottom-most visible
//!       layer's `opacity`/`fillOpacity`; this module applies them, like
//!       §4.6 and like the reference export. The two therefore agree
//!       exactly when the bottom-most visible layer has
//!       `opacity == fillOpacity == 1` — which is every document that has
//!       not dimmed its background layer, and every fixture in the
//!       batteries below.
//!    b. **Mask coverage is NOT the shared routine.** `merge_onto` has
//!       its own `mask_coverage`, which clamps the sampled value BEFORE
//!       density/invert; [`layer_effective_coverage`] clamps AFTER. For
//!       coverage inside `0..=1` the two are bit-identical (each clamp is
//!       a no-op there), so they agree on every mask a u8/u16 coverage
//!       plane can hold; a Gray/f32 mask storing OUT-OF-RANGE coverage is
//!       the one place they diverge.
//!    c. **Visibility.** `merge_onto` reads the layer's OWN `visible`
//!       flag; this module reads `effective_visible` (§4.6 step 1 / §4.7
//!       — own flag AND every enclosing group's), because it walks nested
//!       trees and flatten's caller does not.
//!    d. **Strictness.** `merge_onto` composites a non-NORMAL blend, a
//!       clipped layer and a FEATHERED mask as though they were plain
//!       source-over; this module REFUSES all three typed (below). Where
//!       they differ, this side declines to answer and flatten answers
//!       wrongly — the safe direction, never the reverse.
//!
//!    Where (a)–(d) differ, THIS side matches §4.6 and §12.4's
//!    [NORMATIVE] "a rasterized result is bit-identical to the on-screen
//!    composite"; the flatten side is the outlier. Recorded and escalated
//!    rather than papered over — fixing `image.flatten` moves committed
//!    golden pixels and belongs to whoever owns that row.
//!
//! When the two arithmetics are reconciled project-wide (the recorded
//! divergence), this door moves with `kinds_merge` — one edit, because the
//! per-pixel math has one home.
//!
//! # Laziness
//!
//! [`SamplingComposite`] resolves the participating layer list ONCE (the
//! serve/refuse decision and the `&Buffer` borrows) and then composites
//! **per read**. It never materializes a canvas-sized plane: a clone dab
//! touches a few hundred source pixels, and a canvas-sized f32 plane on a
//! large document is hundreds of MB. Reads outside every participant's
//! content are all-zero (§2.4), exactly as a buffer read is, so the
//! callers' existing off-canvas rules (§45.2 edge policy) are unchanged.
//!
//! # Serve / refuse (QA-F8: serve or refuse typed, never silently drop)
//!
//! SERVED: raster leaves at any depth, pass-through groups, per-layer
//! `opacity`/`fillOpacity`, and ENABLED unfeathered raster masks (§11.2
//! Sub-slice A) — exactly the set `image.flatten` composites today, plus
//! pass-through group nesting.
//!
//! REFUSED typed (`error.command.not_implemented`), each naming its row:
//! non-NORMAL blends and `clipToBelow` (§4.5 kernel table), isolating or
//! decorated groups (§4.7), feathered masks (§11.2 Sub-slice B, the SHARED
//! [`crate::graph::mask_eval::FEATHER_DEFERRED_MSG`]), non-identity layer
//! transforms (§18 resample), and parametric layer kinds — adjustment,
//! smart object, shape, text — which have a LINEAR reference path only.
//! A refusal is never a silent omission: a document whose sampled scope
//! carries an adjustment layer refuses, it does not sample a composite
//! that quietly lacks the adjustment.
//!
//! # `ignoreAdjustments` (§45.2)
//!
//! [`SampleRequest::ignore_adjustments_above`] carries the §45.2 toggle:
//! with `source:"composite"` and the toggle on, every ADJUSTMENT layer
//! strictly above the TARGET layer in z-order is disabled for the sampling
//! composite (so cloning under an adjustment does not double-apply it when
//! the result re-composites through that adjustment). Those layers are
//! then genuinely absent from the sample — which is also why they no
//! longer force the parametric refusal above.

use crate::engine::error::{EngineError, ErrorCode};
use crate::geom::IVec2;
use crate::graph::document::Document;
use crate::graph::id::LayerId;
use crate::graph::mask_eval::{
    layer_effective_coverage, stack_needs_feather, FEATHER_DEFERRED_MSG,
};
use crate::graph::mask_node::MaskNode;
use crate::graph::node::{BlendMode, LayerKind};

/// ONE §4.4 source-over step in STORAGE space — the single per-pixel
/// routine shared by [`crate::engine::kinds_merge`]'s `merge_onto` (the
/// `layer.merge_down` / `layer.merge_visible` / `image.flatten` math) and
/// by [`SamplingComposite::read_pixel_storage`].
///
/// `alpha_scale` is the layer's `opacity · fillOpacity`; `coverage` is its
/// §11.2 effective mask coverage `E` at this pixel. They are kept as TWO
/// arguments, multiplied in this exact left-to-right order, because f32
/// multiplication is not associative — folding them into one weight at the
/// call site would make this a NON-bit-identical extraction of the merge
/// math, and the flatten goldens key on the existing bits.
///
/// `a <= 0` returns `dst` untouched (the merge walk's `continue`).
#[inline]
pub fn source_over_storage(
    dst: [f32; 4],
    src: [f32; 4],
    alpha_scale: f32,
    coverage: f32,
) -> [f32; 4] {
    let a = src[3] * alpha_scale * coverage;
    if a <= 0.0 {
        return dst;
    }
    let scale = a / src[3].max(f32::EPSILON);
    let ia = 1.0 - a;
    [
        src[0] * scale + dst[0] * ia,
        src[1] * scale + dst[1] * ia,
        src[2] * scale + dst[2] * ia,
        a + dst[3] * ia,
    ]
}

/// Which slice of the tree the sampling composite spans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleScope {
    /// The whole document (§45.2 `all`; §13.3 `sampleAllLayers`).
    Document,
    /// This layer and everything BELOW it in z-order (§45.2
    /// `current_and_below`). The z-order is [`Document::z_order_walk`]'s
    /// bottom-first listing, truncated after the named layer.
    AtOrBelow(LayerId),
}

/// A resolved sampling request.
#[derive(Clone, Copy, Debug)]
pub struct SampleRequest {
    pub scope: SampleScope,
    /// §45.2 `ignoreAdjustments`: when `Some(target)`, every ADJUSTMENT
    /// layer STRICTLY ABOVE `target` in z-order is disabled for this
    /// composite. `None` = the toggle is off (adjustments participate,
    /// which today means they refuse typed).
    pub ignore_adjustments_above: Option<LayerId>,
}

/// One participant, resolved once for the whole sampling session.
struct SamplingLayer<'a> {
    pixels: &'a crate::buffer::Buffer,
    /// ENABLED masks only (§11.2 — a disabled mask is the `E = 1` no-op),
    /// RESOLVED once at construction: `read_pixel_storage` is a per-pixel
    /// call, so resolving there would rebuild the stack for every sample.
    masks: crate::graph::mask_eval::ResolvedMasks<'a>,
    /// `opacity · fillOpacity` (§4.6 step 2).
    alpha_scale: f32,
}

/// The lazy sampling composite: a bottom-to-top participant list plus the
/// storage posture a caller must read it with.
pub struct SamplingComposite<'a> {
    layers: Vec<SamplingLayer<'a>>,
    premultiplied: bool,
}

impl SamplingComposite<'_> {
    /// Composite this document pixel bottom-to-top, in STORAGE space.
    /// Absent content reads all-zero (§2.4), exactly as a buffer read.
    pub fn read_pixel_storage(&self, p: IVec2) -> [f32; 4] {
        let mut out = [0.0f32; 4];
        for l in &self.layers {
            let src = l.pixels.read_pixel_f32(p);
            let cov = layer_effective_coverage(&l.masks, p);
            out = source_over_storage(out, src, l.alpha_scale, cov);
        }
        out
    }

    /// The storage posture of the composited value — the BOTTOM-MOST
    /// participant's, which is exactly what `image.flatten` leaves behind
    /// (its receiver is the lowest visible layer and keeps its own buffer
    /// flags). Every participant is known to agree: a scope whose layers
    /// disagree refuses in [`resolve_sampling_composite`] rather than
    /// compositing two conventions together. An empty composite reports
    /// `false`; every read is all-zero, for which the flag is immaterial.
    pub fn premultiplied(&self) -> bool {
        self.premultiplied
    }

    /// How many layers this composite spans (0 = an empty scope: every
    /// read is transparent).
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }
}

/// How many nodes hang below `id` in the tree (0 for a leaf). Used to run
/// the `AtOrBelow` cut past a group's whole depth-first subtree; iterative
/// so a deeply nested tree cannot blow the stack.
fn subtree_len(doc: &Document, id: LayerId) -> usize {
    let mut n = 0usize;
    let mut stack: Vec<LayerId> = doc.children_of(id).map(<[_]>::to_vec).unwrap_or_default();
    while let Some(k) = stack.pop() {
        n += 1;
        if let Some(kids) = doc.children_of(k) {
            stack.extend_from_slice(kids);
        }
    }
    n
}

/// Resolve the §45.2/§13.3 sampling composite over `doc`.
///
/// Errors are typed and name their row (module doc, "Serve / refuse").
/// A layer the walk cannot composite is REFUSED, never dropped.
pub fn resolve_sampling_composite<'a>(
    doc: &'a Document,
    req: &SampleRequest,
) -> Result<SamplingComposite<'a>, EngineError> {
    let refuse = |what: String| EngineError::new(ErrorCode::NotImplemented, what);
    let order = doc.z_order_walk();

    // Scope cut: `AtOrBelow` keeps the bottom-first prefix through the
    // named layer inclusive. `z_order_walk` is depth-first and lists a
    // group IMMEDIATELY BEFORE its own subtree, so:
    //
    // * for a LEAF the prefix is `..=i` and is coherent even when the leaf
    //   is nested inside a group (its siblings above it, and every node
    //   above the enclosing group, fall outside);
    // * for a GROUP the prefix must run past its WHOLE SUBTREE, or the
    //   "armed layer and everything below it" would silently EXCLUDE the
    //   armed layer's own content — the group node itself carries no
    //   pixels. That silent omission is exactly what the serve/refuse
    //   rule forbids, so the cut is extended rather than left short.
    let cutoff = match req.scope {
        SampleScope::Document => order.len(),
        SampleScope::AtOrBelow(id) => {
            let i = order.iter().position(|x| *x == id).ok_or_else(|| {
                EngineError::bad_target(format!(
                    "sampling scope layer {id} does not resolve in the tree (§45.2 \
                     current_and_below)"
                ))
            })?;
            i + 1 + subtree_len(doc, id)
        }
    };
    // §45.2 ignoreAdjustments: the z-order index of the TARGET layer;
    // adjustments STRICTLY above it are disabled.
    let ignore_above = match req.ignore_adjustments_above {
        None => None,
        Some(id) => Some(order.iter().position(|x| *x == id).ok_or_else(|| {
            EngineError::bad_target(format!(
                "ignoreAdjustments target layer {id} does not resolve in the tree (§45.2)"
            ))
        })?),
    };

    let mut layers: Vec<SamplingLayer<'a>> = Vec::new();
    for (i, id) in order.iter().take(cutoff).enumerate() {
        let id = *id;
        // §4.6 step 1 / §4.7: own flag AND every enclosing group visible.
        // Checked FIRST so a HIDDEN decorated group (which the composite
        // never touches) never forces a refusal below.
        if !doc.effective_visible(id) {
            continue;
        }
        let Some(node) = doc.layer(id) else { continue };
        let masks: Vec<&MaskNode> = node
            .masks()
            .iter()
            .filter_map(|m| doc.mask(*m))
            .filter(|m| m.enabled())
            .collect();

        if node.is_group() {
            // A PASS-THROUGH group composites its children in place
            // (§4.7); `z_order_walk` already lists them right after it,
            // so there is nothing to push. Any DECORATION makes the group
            // an isolated unit with its own intermediate — the §4.7
            // rows 1-4 row, refused typed.
            if node.isolated()
                || node.blend() != BlendMode::Normal
                || node.opacity() != 1.0
                || node.fill_opacity() != 1.0
                || node.clip_to_below()
                || !masks.is_empty()
            {
                return Err(refuse(format!(
                    "group \"{}\" in the sampling scope isolates (§4.7 rows 1-4); the \
                     sampling composite implements §4.4 source-over over pass-through \
                     groups only — refused typed, never silently flattened",
                    node.name()
                )));
            }
            continue;
        }

        if node.blend() != BlendMode::Normal {
            return Err(refuse(format!(
                "layer \"{}\" in the sampling scope carries blend {:?}; the sampling \
                 composite implements §4.4 source-over only (the §4.5 kernel table is the \
                 render-side shared math) — refused typed, never silently normalized",
                node.name(),
                node.blend()
            )));
        }
        if node.clip_to_below() {
            return Err(refuse(format!(
                "layer \"{}\" in the sampling scope clips to below; clipped-group \
                 compositing arrives with the compositor clipping row — refused typed",
                node.name()
            )));
        }
        if !node.transform().is_identity() {
            return Err(refuse(format!(
                "layer \"{}\" in the sampling scope carries a non-identity transform; \
                 transformed-layer sampling is the §18 resample row — refused typed",
                node.name()
            )));
        }
        if stack_needs_feather(&masks) {
            return Err(refuse(format!(
                "layer \"{}\" {FEATHER_DEFERRED_MSG}",
                node.name()
            )));
        }

        match node.kind() {
            LayerKind::Raster(buf) => layers.push(SamplingLayer {
                pixels: buf,
                masks: crate::graph::mask_eval::ResolvedMasks::resolve(&masks),
                alpha_scale: node.opacity() * node.fill_opacity(),
            }),
            LayerKind::Adjustment(_) => {
                // §45.2 ignoreAdjustments: adjustments STRICTLY above the
                // target are disabled for this composite — genuinely
                // absent, so they neither apply nor refuse.
                if ignore_above.is_some_and(|t| i > t) {
                    continue;
                }
                return Err(refuse(format!(
                    "adjustment layer \"{}\" is inside the sampling scope; the §12.1 \
                     parametric evaluation has a LINEAR reference path only \
                     ({}), which would fork the sampled pixels away from the \
                     storage-space flatten this door mirrors — refused typed, never a \
                     composite that silently omits the adjustment. §45.2's \
                     ignoreAdjustments disables adjustments ABOVE the target",
                    node.name(),
                    "crate::io::png_io::composite_plane_f32"
                )));
            }
            other => {
                return Err(refuse(format!(
                    "layer \"{}\" is a \"{}\" layer inside the sampling scope; parametric \
                     kinds have a LINEAR reference composite path only — refused typed, \
                     never omitted (QA-F8). Rasterize it (§12.4) to sample it",
                    node.name(),
                    other.kind_str()
                )));
            }
        }
    }

    // §6.4 storage posture: a layer LOADED from straight-alpha data keeps
    // `premultiplied = false` (`Buffer::set_premultiplied_loaded` — the
    // package reader at `io::package`, the PNG reader at `io::png_io`),
    // while a freshly created one is premultiplied. TWO distinct hazards
    // follow, and BOTH refuse typed rather than sample wrong pixels.
    //
    // (1) MIXED: one document can hold both conventions at once. The
    //     composite reports ONE posture to its caller, so a scope whose
    //     layers disagree has no honest answer to report.
    let premultiplied = layers
        .first()
        .map(|l| l.pixels.premultiplied())
        .unwrap_or(false);
    if layers
        .iter()
        .any(|l| l.pixels.premultiplied() != premultiplied)
    {
        return Err(refuse(
            "the sampling scope mixes premultiplied and straight-alpha layer storage \
             (§6.4); one storage-space source-over cannot span both conventions and \
             the composite reports a single posture — refused typed, never a silently \
             mis-composited sample. Convert the straight-alpha layers on load or \
             narrow the sample scope"
                .to_string(),
        ));
    }
    // (2) UNIFORMLY STRAIGHT-ALPHA, MORE THAN ONE LAYER. Agreeing on
    //     `premultiplied = false` is NOT enough: [`source_over_storage`]
    //     IS the §4.4 premultiplied step, so stacking two straight-alpha
    //     layers through it composites the wrong arithmetic — and reports
    //     `premultiplied() == false`, after which the caller's
    //     straight→premultiplied conversion multiplies by alpha a second
    //     time. Silently wrong pixels, which is precisely the class this
    //     door exists to refuse (QA-F8).
    //
    //     ONE layer is exempt and stays SERVED, on the invariant this
    //     module is built on: source-over onto a transparent accumulator
    //     returns the source verbatim (`single_source_over_transparent_
    //     is_bit_exact_identity`), so with one participant the posture
    //     never enters the arithmetic and the value passes through
    //     bit-exact in whatever convention it was stored. That exemption
    //     is what keeps invariant 1 — `sampleAllLayers` must not fork on a
    //     one-layer document — true for straight-alpha documents too.
    //
    //     `merge_onto` has the SAME hole and does not refuse (it runs the
    //     premultiplied step over whatever postures the tree holds), so
    //     this door is deliberately STRICTER than `image.flatten` here.
    //     Recorded and escalated rather than mirrored: converging on the
    //     flatten side means either converting on load or teaching
    //     `merge_onto` the posture, and both move committed golden pixels
    //     — that belongs to whoever owns the flatten row, not to this one.
    if !premultiplied && layers.len() > 1 {
        return Err(refuse(format!(
            "the sampling scope spans {} straight-alpha (un-premultiplied) layers \
             (§6.4); the storage-space source-over this composite runs is the §4.4 \
             PREMULTIPLIED step, so stacking straight-alpha layers through it would \
             silently produce neither convention — refused typed, never a silently \
             mis-composited sample. A single straight-alpha layer still serves (it \
             passes through bit-exact); convert on load or narrow the sample scope",
            layers.len()
        )));
    }
    Ok(SamplingComposite {
        layers,
        premultiplied,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::geom::IRect;
    use crate::graph::document::DocumentGeometry;
    use crate::graph::id::DocumentId;
    use crate::graph::node::LayerNode;
    use crate::tile::model::ColorModelKind;
    use crate::{ChannelKind, ColorState, ProfileId};

    /// A 16×16 document with two opaque raster leaves, `lo` then `hi`.
    fn doc_2x_raster() -> (Document, LayerId, LayerId) {
        let color = ColorState {
            working: ProfileId::nil(),
            mode: ColorModelKind::Rgba,
            depth: ChannelKind::U8,
            rendering_intent: None,
            black_point_compensation: None,
        };
        let mut doc = Document::new(
            DocumentId::new(),
            LayerId::new(),
            DocumentGeometry::new(16, 16),
            color,
        )
        .unwrap();
        let root = doc.root();
        let mk = |doc: &Document, v: [f32; 4]| {
            let mut b = Buffer::new(doc.color().depth, doc.color().mode, doc.color().working);
            b.fill_rect(IRect::new(0, 0, 16, 16), v);
            b
        };
        let lo_buf = mk(&doc, [0.5, 0.0, 0.0, 1.0]);
        let hi_buf = mk(&doc, [0.0, 0.0, 0.5, 1.0]);
        let lo = doc
            .insert_child(root, 0, LayerNode::new_raster(LayerId::new(), "lo", lo_buf))
            .unwrap();
        let hi = doc
            .insert_child(root, 1, LayerNode::new_raster(LayerId::new(), "hi", hi_buf))
            .unwrap();
        (doc, lo, hi)
    }

    fn document_scope() -> SampleRequest {
        SampleRequest {
            scope: SampleScope::Document,
            ignore_adjustments_above: None,
        }
    }

    /// §6.4: one document CAN hold both storage conventions — the package
    /// and PNG readers call `Buffer::set_premultiplied_loaded(false)` for
    /// straight-alpha sources while a freshly created layer is
    /// premultiplied. The composite runs ONE premultiplied source-over and
    /// reports ONE posture, so a scope spanning both must REFUSE rather
    /// than silently mis-composite. Paired with its GREEN control, and
    /// with the narrower scope that still serves.
    #[test]
    fn mixed_storage_posture_in_one_scope_refuses_typed() {
        // Control: matched posture serves, and the composite reports it.
        let (doc, _, _) = doc_2x_raster();
        let c = resolve_sampling_composite(&doc, &document_scope())
            .expect("control: two premultiplied layers compose");
        assert_eq!(c.layer_count(), 2);
        assert!(c.premultiplied(), "control posture is premultiplied");

        // Perturbation: the UPPER layer arrives straight-alpha.
        let (mut doc, lo, hi) = doc_2x_raster();
        doc.layer_mut(hi)
            .unwrap()
            .pixels_mut()
            .unwrap()
            .set_premultiplied_loaded(false);
        let err = match resolve_sampling_composite(&doc, &document_scope()) {
            Err(e) => e,
            Ok(_) => panic!("a scope mixing conventions must refuse"),
        };
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert!(
            err.message.contains("straight-alpha"),
            "the refusal must name its row; got: {}",
            err.message
        );

        // …and narrowing the scope BELOW the straight-alpha layer serves
        // again, so the refusal is about the SPAN and not about the
        // document carrying such a layer at all.
        resolve_sampling_composite(
            &doc,
            &SampleRequest {
                scope: SampleScope::AtOrBelow(lo),
                ignore_adjustments_above: None,
            },
        )
        .expect("a scope that excludes the straight-alpha layer still serves");
    }

    /// §6.4 hazard (2): a scope where every layer AGREES on
    /// `premultiplied = false` is still not composable — the shared
    /// per-pixel step IS the §4.4 premultiplied one, so stacking two
    /// straight-alpha layers through it is silently wrong arithmetic that
    /// the caller then premultiplies a second time. Refused typed.
    ///
    /// Three legs, so the refusal cannot be over- or under-broad:
    ///   (a) the SAME two layers in the premultiplied posture serve (the
    ///       green control — the refusal is the posture's doing, not the
    ///       fixture's);
    ///   (b) both straight-alpha → typed refusal naming its row;
    ///   (c) narrowing to ONE straight-alpha layer SERVES, and the value
    ///       it reads back is BIT-identical to that layer's own storage —
    ///       which is why the one-layer exemption is sound rather than
    ///       merely tolerated, and is what keeps invariant 1 true for
    ///       straight-alpha documents.
    #[test]
    fn uniformly_straight_alpha_multi_layer_scope_refuses_typed() {
        let p = IVec2::new(4, 4);

        // (a) control: the same two layers, premultiplied.
        let (doc, _, _) = doc_2x_raster();
        resolve_sampling_composite(&doc, &document_scope())
            .expect("control: two premultiplied layers compose");

        // (b) perturbation: BOTH layers straight-alpha.
        let (mut doc, lo, hi) = doc_2x_raster();
        for id in [lo, hi] {
            doc.layer_mut(id)
                .unwrap()
                .pixels_mut()
                .unwrap()
                .set_premultiplied_loaded(false);
        }
        let err = match resolve_sampling_composite(&doc, &document_scope()) {
            Err(e) => e,
            Ok(_) => panic!(
                "two AGREEING straight-alpha layers must still refuse — agreeing on \
                 the wrong convention is not the same as being composable"
            ),
        };
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert!(
            err.message.contains("straight-alpha"),
            "the refusal must name its row; got: {}",
            err.message
        );

        // (c) ONE straight-alpha layer still serves, bit-exact.
        let c = resolve_sampling_composite(
            &doc,
            &SampleRequest {
                scope: SampleScope::AtOrBelow(lo),
                ignore_adjustments_above: None,
            },
        )
        .expect("a SINGLE straight-alpha layer passes through and must still serve");
        assert_eq!(c.layer_count(), 1);
        assert!(
            !c.premultiplied(),
            "…reporting the posture it actually holds"
        );
        let stored = doc.layer(lo).unwrap().pixels().unwrap().read_pixel_f32(p);
        let read = c.read_pixel_storage(p);
        for k in 0..4 {
            assert_eq!(
                read[k].to_bits(),
                stored[k].to_bits(),
                "channel {k}: a one-layer composite must return the layer's own \
                 storage BIT-exact, whatever its posture — that identity is the \
                 whole justification for the exemption above"
            );
        }
    }

    /// The `AtOrBelow` cut runs past a GROUP's whole depth-first subtree
    /// (`z_order_walk` lists a group immediately BEFORE its children), so
    /// arming a group samples the group's own content. Pinned at the core
    /// door as well as through the retouch kinds, because the off-by-one
    /// lives here.
    #[test]
    fn at_or_below_a_group_spans_its_subtree() {
        let (mut doc, lo, _hi) = doc_2x_raster();
        let root = doc.root();
        let g = doc
            .insert_child(root, 2, LayerNode::new_group(LayerId::new(), "g", false))
            .unwrap();
        let mut b = Buffer::new(doc.color().depth, doc.color().mode, doc.color().working);
        b.fill_rect(IRect::new(0, 0, 16, 16), [0.0, 0.25, 0.0, 1.0]);
        let inner = doc
            .insert_child(g, 0, LayerNode::new_raster(LayerId::new(), "inner", b))
            .unwrap();
        assert_eq!(
            doc.z_order_walk().last(),
            Some(&inner),
            "the group's child must be listed AFTER the group node"
        );

        let span = |scope| {
            resolve_sampling_composite(
                &doc,
                &SampleRequest {
                    scope,
                    ignore_adjustments_above: None,
                },
            )
            .unwrap()
            .layer_count()
        };
        assert_eq!(span(SampleScope::AtOrBelow(lo)), 1, "lo alone");
        assert_eq!(
            span(SampleScope::AtOrBelow(g)),
            3,
            "arming the GROUP spans lo + hi + the group's own child — a cut that \
             stopped at the group node would report 2 and silently drop the child"
        );
        assert_eq!(
            span(SampleScope::Document),
            3,
            "…which is the whole document"
        );
    }

    /// `source_over_storage` over a TRANSPARENT accumulator at full weight
    /// returns the source verbatim — the invariant that makes the
    /// `sampleAllLayers` tri-state agree on a one-layer document (module
    /// doc, invariant 1). Asserted on RAW BITS, not epsilon.
    #[test]
    fn single_source_over_transparent_is_bit_exact_identity() {
        let cases: [[f32; 4]; 8] = [
            [0.0, 0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0, 1.0],
            [0.25, 0.5, 0.75, 1.0],
            [0.1, 0.2, 0.3, 0.4],
            // The two smallest non-zero u8 storage steps (1/255, 2/255) —
            // the whole reachable low end of an 8-bit document.
            [0.003_921_569, 0.996_078_4, 0.5, 0.003_921_569],
            [0.003_921_569, 0.996_078_4, 0.5, 0.007_843_138],
            [12.5, 0.0, -3.0, 1.0],        // HDR / out-of-gamut f32 storage
            [0.5, 0.5, 0.5, f32::EPSILON], // the guard boundary, inclusive
        ];
        for src in cases {
            let out = source_over_storage([0.0; 4], src, 1.0, 1.0);
            if src[3] <= 0.0 {
                assert_eq!(out, [0.0; 4], "zero-alpha source contributes nothing");
                continue;
            }
            for k in 0..4 {
                assert_eq!(
                    out[k].to_bits(),
                    src[k].to_bits(),
                    "channel {k} of {src:?} must round-trip BIT-exact, got {out:?}"
                );
            }
        }
    }

    /// The boundary of invariant 1, pinned rather than papered over: the
    /// `src[3].max(f32::EPSILON)` guard the merge math has always carried
    /// means an alpha BELOW `f32::EPSILON` (≈1.19e-7) does NOT round-trip
    /// — color is divided by EPSILON instead of by alpha. Unreachable in
    /// u8/u16 storage (the smallest non-zero step is 1/255 ≈ 3.9e-3) and
    /// vanishing in f32; recorded here so a future reader knows the
    /// invariant's exact reach and this test reds if the guard changes.
    #[test]
    fn sub_epsilon_alpha_is_the_known_boundary_of_the_identity() {
        let src = [0.5, 0.5, 0.5, f32::MIN_POSITIVE];
        let out = source_over_storage([0.0; 4], src, 1.0, 1.0);
        assert_eq!(
            out[3].to_bits(),
            src[3].to_bits(),
            "alpha itself is always exact — only color rides the guard"
        );
        assert_ne!(
            out[0].to_bits(),
            src[0].to_bits(),
            "sub-EPSILON alpha must still exercise the guard; if this stops \
             reproducing the guard changed and the module doc needs updating"
        );
        assert!(
            f32::MIN_POSITIVE < f32::EPSILON,
            "the case must actually be below the guard"
        );
    }

    /// Zero weight (hidden-equivalent, zero opacity, zero coverage) leaves
    /// the accumulator untouched — the merge walk's `continue`.
    #[test]
    fn zero_weight_is_the_identity_on_dst() {
        let dst = [0.2, 0.4, 0.6, 0.8];
        let src = [1.0, 0.0, 0.0, 1.0];
        assert_eq!(source_over_storage(dst, src, 0.0, 1.0), dst);
        assert_eq!(source_over_storage(dst, src, 1.0, 0.0), dst);
        assert_eq!(source_over_storage(dst, [0.0; 4], 1.0, 1.0), dst);
    }

    /// The two-argument weight is NOT folded: this pins the exact
    /// multiplication order the merge walk uses, so the extraction stays
    /// bit-identical. Chosen values where `(a·b)·c != a·(b·c)` in f32.
    #[test]
    fn weight_multiplication_order_matches_the_merge_walk() {
        let alpha_scale = 0.1f32;
        let coverage = 0.7f32;
        let sa = 0.3f32;
        let folded = sa * (alpha_scale * coverage);
        let walked = sa * alpha_scale * coverage;
        assert_ne!(
            folded.to_bits(),
            walked.to_bits(),
            "the chosen constants must actually expose the non-associativity, \
             else this test cannot fail"
        );
        let out = source_over_storage([0.0; 4], [1.0, 1.0, 1.0, sa], alpha_scale, coverage);
        assert_eq!(
            out[3].to_bits(),
            walked.to_bits(),
            "source_over_storage must multiply src.a · alpha_scale · coverage in THAT order"
        );
    }
}
