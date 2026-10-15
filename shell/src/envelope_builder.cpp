// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §9.6/§10.2/§10.3; contracts/commands/{doc.new,brush.stroke,
// eraser.stroke}.yaml.

#include "envelope_builder.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QtGlobal>

#include <cmath>

namespace appname::shell {

QString UuidSource::next()
{
    if (!m_deterministic)
        return QUuid::createUuid().toString(QUuid::WithoutBraces);
    // splitmix64 over (seed, counter): stable across runs and platforms.
    auto mix = [](quint64 z) {
        z += 0x9e3779b97f4a7c15ULL;
        z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
        z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
        return z ^ (z >> 31);
    };
    const quint64 hi = mix(m_seed ^ m_counter);
    const quint64 lo = mix(m_counter + 0x632be59bd9b4e019ULL + m_seed);
    ++m_counter;
    QUuid u(static_cast<uint>(hi >> 32),
            static_cast<ushort>(hi >> 16),
            static_cast<ushort>((hi & 0x0fffULL) | 0x4000ULL), // version 4
            static_cast<uchar>(((lo >> 56) & 0x3f) | 0x80),    // RFC-4122 variant
            static_cast<uchar>(lo >> 48),
            static_cast<uchar>(lo >> 40),
            static_cast<uchar>(lo >> 32),
            static_cast<uchar>(lo >> 24),
            static_cast<uchar>(lo >> 16),
            static_cast<uchar>(lo >> 8),
            static_cast<uchar>(lo));
    return u.toString(QUuid::WithoutBraces);
}

QByteArray buildDocNewEnvelope(const QString &envelopeId,
                               int width,
                               int height,
                               const QString &background)
{
    QJsonObject params;
    params.insert(QLatin1String("width"), width);
    params.insert(QLatin1String("height"), height);
    params.insert(QLatin1String("background"), background);

    QJsonObject envelope;
    envelope.insert(QLatin1String("schema"), QLatin1String("appname.command.v1"));
    envelope.insert(QLatin1String("id"), envelopeId);
    envelope.insert(QLatin1String("kind"), QLatin1String("doc.new"));
    envelope.insert(QLatin1String("params"), params);
    return QJsonDocument(envelope).toJson(QJsonDocument::Compact);
}

namespace {

QJsonArray vec2(const QPointF &p)
{
    return QJsonArray{p.x(), p.y()};
}

QJsonObject cubicSeg(const QPointF &p0, const QPointF &c1, const QPointF &c2,
                     const QPointF &p1)
{
    QJsonObject seg;
    seg.insert(QLatin1String("p0"), vec2(p0));
    seg.insert(QLatin1String("c1"), vec2(c1));
    seg.insert(QLatin1String("c2"), vec2(c2));
    seg.insert(QLatin1String("p1"), vec2(p1));
    return seg;
}

// One segment between two anchors: handleOut -> c1; the destination's
// MIRRORED handleIn -> c2; no-handle anchors get collinear 1/3-2/3
// controls (straight, non-degenerate).
QJsonObject segmentBetween(const PenAnchor &a, const PenAnchor &b)
{
    const QPointF p0 = a.anchor;
    const QPointF p1 = b.anchor;
    const QPointF c1 = a.hasHandle ? a.handleOut : p0 + (p1 - p0) / 3.0;
    const QPointF handleIn = b.anchor - (b.handleOut - b.anchor);
    const QPointF c2 = b.hasHandle ? handleIn : p0 + 2.0 * (p1 - p0) / 3.0;
    return cubicSeg(p0, c1, c2, p1);
}

} // namespace

QJsonObject buildVectorPathJson(const QVector<PenAnchor> &anchors, bool closed)
{
    // §20.1 invariant 3: no degenerate segments — and the stage-6 check
    // rejects the WHOLE path, i.e. the whole pen session (review V-D1).
    // Natural gestures reach exact-duplicate consecutive anchors (two
    // clicks on one snap target; the double-click-to-close pair appends
    // a duplicate before the commit fires). A handle-less duplicate of
    // its predecessor would emit p0==c1==c2==p1 — drop it; a HANDLED
    // anchor at the same point forms a valid tiny loop (not all four
    // points coincide) and is kept. For closed paths a trailing
    // handle-less duplicate of the FIRST anchor would make the closure
    // segment degenerate — drop it too (the closure segment supplies the
    // return to the first anchor).
    QVector<PenAnchor> pts;
    for (const PenAnchor &a : anchors) {
        if (!pts.isEmpty() && !a.hasHandle && a.anchor == pts.constLast().anchor)
            continue;
        pts.append(a);
    }
    if (closed && pts.size() >= 2 && !pts.constLast().hasHandle
        && pts.constLast().anchor == pts.constFirst().anchor)
        pts.removeLast();
    QJsonArray segments;
    for (int i = 1; i < pts.size(); ++i)
        segments.append(segmentBetween(pts.at(i - 1), pts.at(i)));
    if (closed && pts.size() >= 2)
        segments.append(segmentBetween(pts.constLast(), pts.constFirst()));
    QJsonObject subpath;
    subpath.insert(QLatin1String("segments"), segments);
    subpath.insert(QLatin1String("closed"), closed);
    QJsonObject path;
    path.insert(QLatin1String("subpaths"), QJsonArray{subpath});
    path.insert(QLatin1String("fillRule"), QLatin1String("nonzero"));
    return path;
}

QByteArray buildCommandEnvelope(const QString &envelopeId,
                                const QString &kind,
                                const QJsonObject &target,
                                const QJsonObject &params,
                                const QString &mergeKey)
{
    QJsonObject envelope;
    envelope.insert(QLatin1String("schema"), QLatin1String("appname.command.v1"));
    envelope.insert(QLatin1String("id"), envelopeId);
    envelope.insert(QLatin1String("kind"), kind);
    if (!target.isEmpty())
        envelope.insert(QLatin1String("target"), target);
    envelope.insert(QLatin1String("params"), params);
    if (!mergeKey.isEmpty())
        envelope.insert(QLatin1String("mergeKey"), mergeKey);
    return QJsonDocument(envelope).toJson(QJsonDocument::Compact);
}

QByteArray buildStrokeEnvelope(const QString &envelopeId, const StrokeParams &p)
{
    QJsonObject params;
    params.insert(QLatin1String("brushPreset"), p.brushPreset);
    params.insert(QLatin1String("sizePx"), p.sizePx);
    params.insert(QLatin1String("opacity"), p.opacity);
    params.insert(QLatin1String("flow"), p.flow);
    // EMITTED ONLY WHEN NON-EMPTY. Every stroke schema is
    // additionalProperties:false, so an unconditional `mode` would be a
    // stage-3 rejection for brush/eraser/smudge — the same reason
    // hardness is brush-only below.
    if (!p.mode.isEmpty())
        params.insert(QLatin1String("mode"), p.mode);
    // Row 5 (see the StrokeParams field note): kind-guarded, true-only.
    if (p.sampleAllLayers
        && (p.kind == QLatin1String("heal.stroke")
            || p.kind == QLatin1String("clone.stroke"))) {
        params.insert(QLatin1String("sampleAllLayers"), true);
    }
    // B-02, AND IT REPAIRS A LATENT DEFECT ROW 5 LEFT (C-678). The §45.2
    // "all" pair was emitted only INSIDE the sampleAllLayers branch, and
    // `offset` was emitted nowhere — so a dispatched clone carried one or
    // two MISSING REQUIRED params depending on a boolean. It could not
    // fire while clone never dispatched; dispatch lands here, so the
    // required set is satisfied by construction instead.
    //
    // §45.2 maps the UI sample scope onto source × sampleAllLayers:
    //   all           -> source "composite" + sampleAllLayers true (the
    //                    PAIR: composite+false is the spec's own
    //                    inexpressible row, error.schema.out_of_range)
    //   current layer -> source {layerId} + the flag omitted (default
    //                    false), the layer being the one we paint onto
    if (p.kind == QLatin1String("clone.stroke")) {
        if (p.sampleAllLayers) {
            params.insert(QLatin1String("source"), QLatin1String("composite"));
        } else {
            QJsonObject src;
            src.insert(QLatin1String("layerId"), p.targetLayerId);
            params.insert(QLatin1String("source"), src);
        }
        params.insert(QLatin1String("offset"),
                      QJsonArray{p.cloneOffset.x(), p.cloneOffset.y()});
    }
    if (p.kind == QLatin1String("brush.stroke")) {
        // Item 4: the hardness override is a brush.stroke param ONLY
        // (eraser's contract un-extended; additionalProperties:false).
        //
        // O-29: emitted ONLY when the caller actually states a hardness.
        // `hardness` is the one stroke param the contract leaves without a
        // schema default — "absent = preset hardness" — so omitting it
        // hands the decision to `preset.hardness` in the engine, while any
        // emitted number overrides the preset outright. The shell hardcodes
        // kDefaultBrushPreset today, whose hardness is 0.0 and equals the
        // row's own default, which is exactly why an always-insert cannot
        // be caught by any pixel: it is indistinguishable NOW and silently
        // wrong LATER (a preset picker offering hard-round-16, hardness
        // 1.0, would have an untouched row quietly force it back to 0.0).
        // The unset case is therefore not an optimisation — it is the only
        // encoding of "the preset governs".
        if (p.hardness)
            params.insert(QLatin1String("hardness"), *p.hardness);
        // §22.1 color wire shape: model + exactly four comps (trailing
        // comps zero for rgb), values 0..1 straight from the picker.
        QJsonObject color;
        color.insert(QLatin1String("model"), QLatin1String("rgb"));
        QJsonArray comps;
        comps.append(p.color.redF());
        comps.append(p.color.greenF());
        comps.append(p.color.blueF());
        comps.append(0.0);
        color.insert(QLatin1String("comps"), comps);
        params.insert(QLatin1String("color"), color);
        params.insert(QLatin1String("blend"), QLatin1String("normal"));
    }
    params.insert(QLatin1String("seed"), static_cast<qint64>(p.seed & 0x7fffffffffffffffULL));

    QJsonObject target;
    target.insert(QLatin1String("layerId"), p.targetLayerId);

    QJsonObject envelope;
    envelope.insert(QLatin1String("schema"), QLatin1String("appname.command.v1"));
    envelope.insert(QLatin1String("id"), envelopeId);
    envelope.insert(QLatin1String("kind"), p.kind);
    envelope.insert(QLatin1String("target"), target);
    envelope.insert(QLatin1String("params"), params);
    // §9.6 rule 2: samples NEVER inline — the assetId string only.
    envelope.insert(QLatin1String("samples"), p.samplesAssetId);
    return QJsonDocument(envelope).toJson(QJsonDocument::Compact);
}

int unicodeScalarCount(const QString &text)
{
    // Surrogate-pair walk (no allocation): each high+low pair is ONE
    // scalar; unpaired surrogates count as one scalar each (they will
    // round-trip as replacement scalars — the engine validates content).
    int count = 0;
    for (qsizetype i = 0; i < text.size(); ++i) {
        ++count;
        if (text.at(i).isHighSurrogate() && i + 1 < text.size()
            && text.at(i + 1).isLowSurrogate())
            ++i;
    }
    return count;
}

QJsonObject buildTextAddPointParams(const QPointF &origin,
                                    const QString &text,
                                    const QString &newLayerId,
                                    const QJsonObject &style)
{
    QJsonObject params;
    params.insert(QLatin1String("layout"), QLatin1String("point"));
    params.insert(QLatin1String("origin"),
                  QJsonArray{origin.x(), origin.y()});
    params.insert(QLatin1String("text"), text);
    params.insert(QLatin1String("newLayerId"), newLayerId);
    // 11d Phase 1: styledRunStyle carried verbatim; empty = omitted
    // (§2.8 — never serialized null/empty; pre-11d bytes preserved).
    if (!style.isEmpty())
        params.insert(QLatin1String("style"), style);
    return params;
}

QJsonObject buildTextAddParagraphParams(const QRect &rect,
                                        const QString &text,
                                        const QString &newLayerId,
                                        const QJsonObject &style)
{
    QJsonObject r;
    r.insert(QLatin1String("x"), rect.x());
    r.insert(QLatin1String("y"), rect.y());
    r.insert(QLatin1String("w"), rect.width());
    r.insert(QLatin1String("h"), rect.height());
    QJsonObject params;
    params.insert(QLatin1String("layout"), QLatin1String("paragraph"));
    params.insert(QLatin1String("rect"), r);
    params.insert(QLatin1String("text"), text);
    params.insert(QLatin1String("newLayerId"), newLayerId);
    if (!style.isEmpty())
        params.insert(QLatin1String("style"), style);
    return params;
}

QJsonArray buildSingleRunReplacement(const QString &text)
{
    QJsonArray runs;
    if (!text.isEmpty()) { // zero-length runs forbidden (§23.1 invariant)
        QJsonObject run;
        run.insert(QLatin1String("text"), text);
        runs.append(run);
    }
    return runs;
}

QJsonArray buildWholeStringRange(const QString &text)
{
    return QJsonArray{0, unicodeScalarCount(text)};
}

QJsonObject buildPerspectiveQuadParams(const QPointF quad[4])
{
    QJsonArray q;
    for (int i = 0; i < 4; ++i)
        q.append(QJsonArray{quad[i].x(), quad[i].y()});
    QJsonObject params;
    params.insert(QLatin1String("quad"), q);
    return params;
}

QJsonObject buildWarpPresetParams(const QString &kind, double bendPct)
{
    QJsonObject preset;
    preset.insert(QLatin1String("kind"), kind);
    preset.insert(QLatin1String("bendPct"), bendPct);
    QJsonObject params;
    params.insert(QLatin1String("preset"), preset);
    return params;
}

QJsonObject buildFilterApplyParams(const QString &filterId,
                                   const QJsonObject &filterParams,
                                   const QString &blend,
                                   double opacity)
{
    if (filterId.isEmpty())
        return {};
    QJsonObject params;
    params.insert(QLatin1String("filterId"), filterId);
    params.insert(QLatin1String("params"), filterParams);
    if (!blend.isEmpty())
        params.insert(QLatin1String("blend"), blend);
    if (opacity >= 0.0)
        params.insert(QLatin1String("opacity"), opacity);
    return params;
}

QJsonObject buildLiquifyFieldParams(int meshPitch, const QPointF &origin,
                                    int nx, int ny,
                                    const QVector<double> &disp)
{
    const bool pitchOk = meshPitch == 4 || meshPitch == 8 || meshPitch == 16
        || meshPitch == 32;
    if (!pitchOk || nx < 1 || ny < 1)
        return {};
    if (disp.size() != qsizetype(2) * nx * ny)
        return {}; // field-shape cross-field enforced by construction
    QJsonArray d;
    for (double v : disp)
        d.append(v);
    QJsonObject params;
    params.insert(QLatin1String("meshPitch"), meshPitch);
    params.insert(QLatin1String("origin"),
                  QJsonArray{origin.x(), origin.y()});
    params.insert(QLatin1String("size"), QJsonArray{nx, ny});
    params.insert(QLatin1String("disp"), d);
    return params;
}

QJsonObject buildPuppetParams(const QVector<PuppetPin> &pins,
                              const QString &density,
                              int expansionPx,
                              double rigidity)
{
    if (pins.isEmpty())
        return {}; // len >= 1 is the contract floor — no empty envelope
    QJsonArray arr;
    for (const PuppetPin &pin : pins) {
        QJsonObject p;
        p.insert(QLatin1String("pos"),
                 QJsonArray{pin.pos.x(), pin.pos.y()});
        if (pin.hasRotation)
            p.insert(QLatin1String("rotationDeg"), pin.rotationDeg);
        if (pin.depth != 0)
            p.insert(QLatin1String("depth"), pin.depth);
        arr.append(p);
    }
    QJsonObject params;
    params.insert(QLatin1String("pins"), arr);
    // Optional knobs omitted at their sentinel = the contract defaults
    // stay the ENGINE's (never re-stated client-side).
    if (!density.isEmpty())
        params.insert(QLatin1String("density"), density);
    if (expansionPx >= 0)
        params.insert(QLatin1String("expansionPx"), expansionPx);
    if (rigidity >= 0.0)
        params.insert(QLatin1String("rigidity"), rigidity);
    return params;
}

QJsonObject buildWarpMeshParams(int cols, int rows,
                                const QVector<QPointF> &points)
{
    if (cols < 1 || rows < 1)
        return {};
    const qsizetype expected =
        qsizetype(3 * cols + 1) * qsizetype(3 * rows + 1);
    if (points.size() != expected)
        return {}; // §40.6 cross-field enforced by construction
    QJsonArray pts;
    for (const QPointF &p : points)
        pts.append(QJsonArray{p.x(), p.y()});
    QJsonObject mesh;
    mesh.insert(QLatin1String("cols"), cols);
    mesh.insert(QLatin1String("rows"), rows);
    mesh.insert(QLatin1String("points"), pts);
    QJsonObject params;
    params.insert(QLatin1String("mesh"), mesh);
    return params;
}

QJsonObject buildAdjustmentAddParams(const QString &opKind,
                                     const QString &newLayerId,
                                     const QString &newOpId,
                                     const QJsonObject &opParams)
{
    QJsonObject params;
    params.insert(QLatin1String("opKind"), opKind);
    params.insert(QLatin1String("newLayerId"), newLayerId);
    // §9.6 client-generated creation uuid for the OpNode. REQUIRED by the
    // contract's schema, and the reason the shell can address this op
    // later without the engine ever publishing an id for it.
    params.insert(QLatin1String("newOpId"), newOpId);
    // `params` is in the contract's `required` list, so it is emitted even
    // when the adjustment takes none (adjustment.invert) — an omitted key
    // is a stage-3 missing_field, not a default.
    params.insert(QLatin1String("params"), opParams);
    return params;
}

QJsonObject buildOpSetMaskFromSelectionParams(const QString &opId)
{
    QJsonObject params;
    params.insert(QLatin1String("opId"), opId);
    // The `from` arm only — maskId is unreachable through this builder, so
    // the §40.6 oneOf cannot be violated by a caller. (The engine refuses
    // the maskId arm anyway today: OpSetMaskKind::apply answers
    // error.command.not_implemented for it until the §14 mask/channel
    // library lands — read first-hand in kinds_ops.rs, not assumed.)
    params.insert(QLatin1String("from"), QLatin1String("selection"));
    return params;
}

// ---- mask.set_props (Pro-Wave row 6) --------------------------------------

QString maskPropPath(MaskProp prop)
{
    switch (prop) {
    case MaskProp::Enabled:
        return QStringLiteral("enabled");
    case MaskProp::Inverted:
        return QStringLiteral("inverted");
    case MaskProp::Density:
        return QStringLiteral("density");
    case MaskProp::Feather:
        return QStringLiteral("feather");
    case MaskProp::Linked:
        return QStringLiteral("linked");
    }
    return QString();
}

double maskPropMinimum(MaskProp prop)
{
    // contracts/commands/mask.set_props.yaml paramsSchema.
    switch (prop) {
    case MaskProp::Density:
    case MaskProp::Feather:
        return 0.0;
    default:
        return 0.0;
    }
}

double maskPropMaximum(MaskProp prop)
{
    switch (prop) {
    case MaskProp::Density:
        return 1.0;
    case MaskProp::Feather:
        return 1000.0;
    default:
        return 1.0;
    }
}

// MaskNode's own construction defaults (crates/appname-core/src/graph/
// mask_node.rs:48-52) — the "field's documented default" the contract's
// non-finite rule sends NaN to. The contract's own `defaults: {}` is empty
// because these live on the node, not on the command.
static double maskPropDefault(MaskProp prop)
{
    switch (prop) {
    case MaskProp::Density:
        return 1.0;
    case MaskProp::Feather:
        return 0.0;
    default:
        return 0.0;
    }
}

QJsonObject buildMaskSetPropsParams(MaskProp prop, double value)
{
    QJsonObject params;
    const QString path = maskPropPath(prop);
    if (path.isEmpty())
        return params;
    if (prop == MaskProp::Enabled || prop == MaskProp::Inverted
        || prop == MaskProp::Linked) {
        // A bool prop reached the numeric overload: refuse by construction
        // rather than emit a number where the schema demands a boolean
        // (additionalProperties:false makes a mistyped field a stage-3
        // violation, and the contract's own `invalid` case for a mistyped
        // bool is error.schema.missing_field — an unhelpful surface to
        // hand a user when we can simply never build it).
        return {};
    }
    double v = value;
    if (std::isnan(v)) {
        v = maskPropDefault(prop); // §0.3/§40.1 inlined non-finite policy
    } else if (std::isinf(v)) {
        v = v > 0 ? maskPropMaximum(prop) : maskPropMinimum(prop);
    } else {
        v = qBound(maskPropMinimum(prop), v, maskPropMaximum(prop));
    }
    params.insert(path, v);
    return params;
}

QJsonObject buildMaskSetPropsParams(MaskProp prop, bool value)
{
    QJsonObject params;
    if (prop != MaskProp::Enabled && prop != MaskProp::Inverted
        && prop != MaskProp::Linked)
        return params; // a numeric prop reached the bool overload
    params.insert(maskPropPath(prop), value);
    return params;
}

QString maskPropSessionMergeKey(const QString &maskId,
                                const QString &sessionUuid,
                                MaskProp prop)
{
    if (maskId.isEmpty() || sessionUuid.isEmpty())
        return QString();
    const QString path = maskPropPath(prop);
    if (path.isEmpty())
        return QString();
    return QStringLiteral("mask.set_props:") + maskId + QLatin1Char(':')
        + sessionUuid + QLatin1Char('#') + path;
}

} // namespace appname::shell
