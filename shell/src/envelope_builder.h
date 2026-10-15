// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §9.6/§10.2 (appname.command.v1 envelope; client-generated
// object UUIDs so replay is deterministic), §10.3 + contracts
// commands/brush.stroke.yaml + eraser.stroke.yaml (stroke params, the
// samples-as-assetId rule), contracts/commands/doc.new.yaml. D-48: the
// shell mutates ONLY by building these envelopes and applying them.

#pragma once

#include <QByteArray>
#include <QColor>
#include <QJsonArray>
#include <QJsonObject>
#include <QPointF>
#include <QRect>
#include <QVector>
#include <QString>
#include <QUuid>

#include <cstdint>
#include <optional>

namespace appname::shell {

// Default brush-preset AssetId: the engine's built-in soft-round-pressure
// preset (§28.1 minimal resolution until the CP-0.5 resource index).
// Content-independent constant, UUIDv5(APPNAME_NS,
// "preset/builtin/soft-round-pressure") — mechanically derived and
// cross-confirmed against the engine's own resolve_preset output and a
// committed CLI-leg brush.stroke (rule 8: no hand-invented pins). The
// documented built-in alternative is hard-round-16
// (eeced2e4-ee37-587c-a8a0-8fca734089d1); a preset picker arrives with the
// CP-0.5 resource index. The cabi stroke flip landed (assembled engine +
// appname_asset_create ingress): ABI strokes commit against this preset.
inline const QString kDefaultBrushPreset =
    QStringLiteral("7efe96cd-64b6-5e4d-835c-5dd050705583");

// UUID source: random in the app, seeded-deterministic in --screenshot-tour
// (fixed envelope ids keep the tour reproducible; client-generated ids are
// the §9.6 replay-determinism rule).
class UuidSource {
public:
    // Random (QUuid::createUuid).
    UuidSource() = default;
    // Deterministic: uuids derived from (seed, counter) — RFC-4122 v4 shaped.
    explicit UuidSource(quint64 seed)
        : m_deterministic(true)
        , m_seed(seed)
    {
    }
    QString next();

private:
    bool m_deterministic = false;
    quint64 m_seed = 0;
    quint64 m_counter = 0;
};

struct StrokeParams {
    QString kind;             // "brush.stroke" | "eraser.stroke" | …
    // v1.0.1: the STROKE MODE, for the kinds whose contract requires one.
    // dodge_burn.stroke and blur_sharpen.stroke each have `mode` in their
    // REQUIRED set, and the value is not a preference — it is the tool the
    // user already picked, because the palette spells the mode
    // (dodge_burn.dodge / .burn, blur_sharpen.blur / .sharpen) while the
    // WIRE kind is the family. EMPTY = absent, which is correct for every
    // kind whose schema has no `mode` and where additionalProperties:false
    // makes a stray one a stage-3 violation.
    QString mode;
    // Tier-1 row 5: the §45.2/§13.3 sample-scope flag. Emitted KIND-
    // GUARDED — heal.stroke and clone.stroke only, the two stroke kinds
    // in the order's scope whose REGISTERED schemas carry the key
    // (additionalProperties:false makes a stray key a stage-3 violation
    // everywhere else) — and ONLY when true (schema default false, the
    // default-omission rule). On clone.stroke true ALSO emits
    // source:"composite": §45.2's "all" scope travels as the PAIR, and
    // composite+false is the spec's own inexpressible row
    // (error.schema.out_of_range by design). smudge.stroke's schema
    // carries the flag too and is OUTSIDE the order's three tools —
    // a recorded gap, not creep (C-666).
    bool sampleAllLayers = false;
    // B-02: clone.stroke's REQUIRED set is [brushPreset, source, offset]
    // (clone.stroke.yaml:31) — UNCONDITIONALLY, not only under the §45.2
    // "all" pair row 5 wired. `offset` is the ARMED source offset
    // [dx,dy]px (§21.1): with `aligned` defaulting true the contract's
    // invariant is C(P) − P constant until re-arm, so this is exactly
    // anchor − strokeStart, computed once at stroke begin. Meaningless
    // for every other kind and never emitted for one.
    QPointF cloneOffset;
    QString targetLayerId;    // envelope target.layerId
    QString samplesAssetId;   // envelope samples (assetId uuid — NEVER inline)
    QString brushPreset = kDefaultBrushPreset;
    double sizePx = 16.0;     // 0.1..=10000
    // Item 4 / O-29: brush.stroke's hardness OVERRIDE (0..=1, "absent =
    // preset hardness"). Emitted for brush.stroke ONLY — eraser.stroke's
    // contract is un-extended, and additionalProperties:false makes a
    // stray param a stage-3 violation.
    //
    // OPTIONAL BY TYPE, deliberately. brush.stroke.yaml gives `hardness`
    // NO schema default (unlike opacity/flow, which each default to 1):
    // absent means "this command states no hardness", and the engine then
    // uses the preset's own value (appname-tools brush/plan.rs:
    // `params.hardness.unwrap_or(preset.hardness)`); a PRESENT value
    // overrides the preset unconditionally. Carrying that optionality in
    // the C++ type — instead of a double plus a convention — is what stops
    // a later writer from restoring an unconditional insert: no value of
    // this member means "absent" by accident. Unset => the key is omitted
    // from the wire (see buildStrokeEnvelope).
    std::optional<double> hardness; // 0..=1 when set; unset = omit the key
    double opacity = 1.0;     // 0..=1
    double flow = 1.0;        // 0..=1
    QColor color = QColor(0, 0, 0); // brush.stroke only (eraser has no color param)
    quint64 seed = 0;         // per-stroke dynamics seed (recorded, §15.4)
};

// Pen-tool anchor: a point with an optional symmetric handle pair
// (handleIn is the mirror of handleOut about the anchor). Collected by
// the canvas; converted to the contract's cubic-segment JSON here.
struct PenAnchor {
    QPointF anchor;
    QPointF handleOut;
    bool hasHandle = false;
};

// §20.1 VectorPath JSON (vector.path_new / vector.shape_add `path`):
// consecutive anchors become CubicSegs; anchors without handles get
// collinear 1/3-2/3 controls (a straight segment, non-degenerate);
// handleOut drives c1, the NEXT anchor's mirrored handleIn drives c2.
// p0 duplicates the previous segment's p1 VERBATIM (same doubles -> same
// serialized bytes: the §20.1 bitwise-contiguity invariant). closed
// appends the return segment (last anchor -> first anchor, same rules)
// so last.p1 == first.p0 exactly. Needs >= 2 anchors; fillRule nonzero.
QJsonObject buildVectorPathJson(const QVector<PenAnchor> &anchors, bool closed);

// ---- slice T: text-kind param builders (contracts text.*.yaml) ------------
// Unicode SCALAR count (§23.4 / text range family rule): ranges and run
// lengths count scalar values, NOT UTF-16 code units — QString::size()
// over-counts astral chars (the R14 lesson, pinned in tst_text_builders).
int unicodeScalarCount(const QString &text);

// text.add params, layout=point: origin REQUIRED by the §40.7
// cross-field rule — enforced by construction (there is no builder that
// can emit a point layout without one). text defaults empty (the
// contract's own default; the typing surface arrives with the chrome).
// `style` (11d Phase 1): an OPTIONAL styledRunStyle object carried
// VERBATIM (field exactness is the caller's contract duty, the
// standing builder rule); empty = omitted from the wire — the default
// {} keeps every pre-11d call site and the tour byte-identical.
QJsonObject buildTextAddPointParams(const QPointF &origin,
                                    const QString &text,
                                    const QString &newLayerId,
                                    const QJsonObject &style = QJsonObject());

// text.add params, layout=paragraph: rect REQUIRED (IRect, integer px,
// §10.3 conventions) — enforced by construction. `style` as above.
QJsonObject buildTextAddParagraphParams(const QRect &rect,
                                        const QString &text,
                                        const QString &newLayerId,
                                        const QJsonObject &style = QJsonObject());

// text.edit `runs`: FULL-STATE replacement — the concatenation of
// runs[i].text IS the layer's string (§23.1). Zero-length runs are
// forbidden by the run-model invariants, so the empty string is the
// EMPTY ARRAY, never [{text:""}]. v1 surface: one unstyled run.
QJsonArray buildSingleRunReplacement(const QString &text);

// Whole-string [start,end] range for text.set_style/set_paragraph —
// [0, scalarCount] in Unicode scalars (family rule).
QJsonArray buildWholeStringRange(const QString &text);

// ---- transform.warp / transform.perspective param builders ---------------
// transform.perspective `quad`: four destination corners [x,y] in
// TL,TR,BR,BL order (the contract example's winding), doc px.
QJsonObject buildPerspectiveQuadParams(const QPointF quad[4]);

// transform.warp preset arm (§40.6 action-friendly variant): kind is one
// of the 11 contract enum values, bendPct -100..100 — RANGES ARE THE
// CALLER'S contract duty (dialog/tour own exactness, the CP-0.5 rule);
// the builder carries values verbatim.
QJsonObject buildWarpPresetParams(const QString &kind, double bendPct);

// transform.warp mesh arm: WarpGrid over the (3*cols+1)x(3*rows+1)
// Bezier control lattice, points row-major. The exact point count is a
// CONTRACT CROSS-FIELD (§40.6) — enforced by construction: a count
// mismatch returns an EMPTY object (never a silently-wrong envelope).
QJsonObject buildWarpMeshParams(int cols, int rows,
                                const QVector<QPointF> &points);

// ---- transform.puppet param builder --------------------------------------
// Wire semantics per the ENGINE's recorded ruling (kind_impls.rs, §45.5):
// pos is the pin's PLACEMENT (must lie in source content bounds); drag
// destinations are NOT wire-expressible (recorded contract gap, O-TOOLS
// escalation); the expressible deformation is rotationDeg (+depth
// overlap order). rotationDeg ABSENT means free rotation — it must be
// OMITTED from the wire, never serialized null (§2.8).
struct PuppetPin {
    QPointF pos;
    bool hasRotation = false;
    double rotationDeg = 0.0;
    int depth = 0; // contract default 0 — omitted when 0
};

QJsonObject buildPuppetParams(const QVector<PuppetPin> &pins,
                              const QString &density = QString(),
                              int expansionPx = -1,
                              double rigidity = -1.0);

// ---- filter.apply param builders -----------------------------------------
// filter.apply carries filterId + the filter's OWN params object
// (re-validated engine-side against the registered schema, §16.1) and
// an optional §10.3 fade pass. Fade fields are omitted at sentinels so
// the engine's defaults are never re-stated.
QJsonObject buildFilterApplyParams(const QString &filterId,
                                   const QJsonObject &filterParams,
                                   const QString &blend = QString(),
                                   double opacity = -1.0);

// filter.liquify's complete field payload (the engine's registry
// citizen carries the §16.3 mesh-schema field set verbatim — corpus/
// liquify.rs recorded pin): meshPitch in {4,8,16,32}, origin [x,y]
// (doc px, node (0,0)), size [nx,ny] nodes (>=1 each), disp = 2*nx*ny
// row-major (dx,dy) px pairs. The count is ENFORCED BY CONSTRUCTION —
// a mismatch (or a bad pitch/size) returns the empty object.
QJsonObject buildLiquifyFieldParams(int meshPitch, const QPointF &origin,
                                    int nx, int ny,
                                    const QVector<double> &disp);

// ---- Pro-Wave row 1: the OP-mask route on an adjustment -------------------
// contracts/commands/adjustment.add.yaml + op.set_mask.yaml (§12.1, §12.2,
// §40.6). THE ID DIRECTION, probed first-hand and contrary to the filed
// record: an OpNode's id is NOT read back from the engine — §9.6 makes
// creation uuids CLIENT-GENERATED, and adjustment.add's schema puts
// `newOpId` in `required`. The shell therefore AUTHORS the opId it later
// addresses; nothing needs to publish one. (The engine publishes no op
// ids at all — `appname_doc_info_json`'s layer object has no `ops` key —
// which is why an op the shell did NOT create in-session stays
// unaddressable. That residual gap is an O-INT ask, recorded in the lane
// log; it does not block the create-then-mask path these builders serve.)
//
// adjustment.add params. `params` is the ADJUSTMENT's own params object,
// re-validated engine-side against the registered opKind schema (§17.1) —
// carried verbatim, the standing builder rule. Emitted even when empty
// (the schema puts `params` in `required`, and additionalProperties:false
// makes a stray key a stage-3 violation).
QJsonObject buildAdjustmentAddParams(const QString &opKind,
                                     const QString &newLayerId,
                                     const QString &newOpId,
                                     const QJsonObject &opParams = {});

// op.set_mask{from:'selection'} params. The contract's stage-3 oneOf is
// enforced BY CONSTRUCTION: this builder can only emit the `from` arm, so
// no caller can produce the both-present payload that answers
// error.schema.missing_field. `from:'selection'` captures the active
// selection's coverage into the op's mask (§40.6); the mask id is
// ENGINE-derived (a §9.6 deterministic-per-envelope id), so — unlike the
// layer-mask route — the shell mints no maskId here.
QJsonObject buildOpSetMaskFromSelectionParams(const QString &opId);
// ---- mask.set_props param builder (Pro-Wave row 6) ------------------------
// contracts/commands/mask.set_props.yaml, read first-hand: FIVE optional
// params, `additionalProperties:false`, "full-state per present field"
// (§10.3). enabled/inverted/linked bool; density number 0..=1; feather
// number 0..=1000 (px).
//
// ★ ONE PROPERTY PER ENVELOPE, ENFORCED BY CONSTRUCTION. The contract's
// merge key is PROPERTY-SCOPED — `{kind}:{targetId}:{sessionUuid}#{propPath}`
// with "merge key scoped per (maskId, prop)" (§10.5 property-drag session;
// ENGINEERING-SPEC.md:13306). A multi-property envelope has no single
// `propPath` to name, so coalescing a feather drag and a density drag under
// one key would fold two independent undo steps into one. The builder
// therefore takes ONE prop and there is no overload that can emit two.
//
// RANGES ARE CLAMPED HERE, and that is SCHEMA duty, not engine math: the
// bounds below are transcribed from the registered paramsSchema, nothing is
// recomputed. The §11.2 feather Gaussian itself (σ = feather_px/3, the
// FI-10 Option B mapping) is ENGINE-OWNED and is deliberately NOT mirrored
// shell-side — this is a pure C-ABI client.
//
// NOTE ON `feather`'s UNITS: the contract description still reads "Gaussian
// sigma", but that wording predates FI-10 Option B. The landed engine
// (crates/appname-core/src/graph/mask_eval.rs:119-127, marked NORMATIVE)
// makes `feather_px` the **3σ RADIUS**, σ = feather_px/3. It is a px reach,
// not a sigma; no shell surface may call it sigma.
enum class MaskProp { Enabled, Inverted, Density, Feather, Linked };

// The contract `propPath` token for a mask property — also the `#`-suffix of
// the §10.5 merge key. Exactly the registered param name.
QString maskPropPath(MaskProp prop);

// Schema-declared bounds for the numeric props, transcribed from
// contracts/commands/mask.set_props.yaml. Bools return {0,1} unused.
// Exposed so tests can assert the clamp against the contract rather than
// against a repeated literal.
double maskPropMinimum(MaskProp prop);
double maskPropMaximum(MaskProp prop);

// mask.set_props params carrying exactly ONE property.
//
// Non-finite handling mirrors the contract's inlined §0.3/§40.1 policy
// ("numeric params arriving non-finite are never a validation error — NaN
// clamps to the field's documented default, ±Inf to the field's documented
// min/max"). We apply the SAME mapping client-side so the wire never
// carries a non-finite (JSON has no NaN/Inf literal — Qt would serialize
// `null` and the engine would see a stage-3 type violation instead of the
// documented clamp).
QJsonObject buildMaskSetPropsParams(MaskProp prop, double value);
QJsonObject buildMaskSetPropsParams(MaskProp prop, bool value);

// §10.5 property-drag session key: "mask.set_props:{maskId}:{sessionUuid}
// #{propPath}". targetId is the MASK id (the contract's targetResolution
// note: "targetId = the maskId"), NOT the owning layer. Empty when either
// id is empty — an empty mergeKey is omitted from the wire by
// buildCommandEnvelope, which is the correct "no session" behavior.
QString maskPropSessionMergeKey(const QString &maskId,
                                const QString &sessionUuid,
                                MaskProp prop);

// appname.command.v1 envelope bytes (compact JSON).
QByteArray buildDocNewEnvelope(const QString &envelopeId,
                               int width,
                               int height,
                               const QString &background); // transparent|white|black

QByteArray buildStrokeEnvelope(const QString &envelopeId, const StrokeParams &params);

// Generic envelope for the CP-0.5 chrome: kind + target + params carried
// verbatim (dialogs own contract exactness — ranges/enums live in the
// dialog definitions, reviewed against contracts/commands/<kind>.yaml).
// Empty target/mergeKey are omitted from the wire. `params` is emitted
// even when empty ({} — kinds with no params carry an empty object per
// the envelope schema).
QByteArray buildCommandEnvelope(const QString &envelopeId,
                                const QString &kind,
                                const QJsonObject &target,
                                const QJsonObject &params,
                                const QString &mergeKey = QString());

} // namespace appname::shell
