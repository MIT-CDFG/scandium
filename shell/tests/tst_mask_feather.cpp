// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Pro-Wave row 6 — THE §11.2 MASK PROPERTY NET (feather, density,
// enabled, inverted, linked).
//
// Cabi-linked (the tst_selection_gestures / tst_auto_adjust idiom): a
// REAL engine document driven through the REAL DocumentController door.
// Every assertion reads ENGINE truth — composite readback pixels and the
// info door's own mask block — never shell echo state.
//
// ★ WHY THIS ROW IS BUILDABLE AT ALL. Until pro/feather-b, render
// REFUSED any feathered mask with a typed `MaskFeatherDeferred`. That
// variant is DELETED and feather now resolves at the two CPU doors
// (png_io's parametric arm and its reference raster walk) and at the
// render compositor, all three through the ONE shared
// `mask_eval::ResolvedMasks`. Verified first-hand before this net was
// written; `featherIsServedAtTheCompositeDoorNotJustTheCommandDoor`
// keeps it verified mechanically, because a command door that accepts
// while the composite ignores is exactly the failure this row was
// blocked on before.
//
// ★ THE FIXTURE, and why it discriminates. A 160x64 doc; a raster layer
// filled opaque BLACK over a WHITE background; a raster mask built from
// a rect selection covering x in [0,80). So the composite is black on
// the left, white on the right, with a HARD STEP at x=80. A step edge is
// the one fixture where a Gaussian's parameters are directly readable
// off the output: the blur's REACH from the step is exactly the kernel's
// truncation radius, so a wrong sigma constant moves a measurable
// boundary rather than merely changing some pixels.
//
// ★ THE TRAP THIS NET IS BUILT AROUND. §11.2 originally read
// `GaussianBlur(sigma = feather_px)`. FI-10 Option B changed the mapping
// to `sigma = feather_px/3`, making feather_px the 3-sigma RADIUS, so
// that ceil(3*sigma) == ceil(feather_px) == Rmask (the dirty/ROI margin
// propagate_roi already used). The registered contract STILL describes
// `feather` as "Gaussian sigma" — stale wording. A smoke test ("the
// composite changed") passes under BOTH readings. `featherReachIs...`
// below fails under the old one, because the two differ by 3x in reach
// and the fixture is sized so that 3x lands on the far probe.

#include "announcer.h"
#include "document_controller.h"
#include "envelope_builder.h"
#include "i18n_catalog.h"

#include <QFile>
#include <QFileInfo>
#include <QGuiApplication>
#include <QImage>
#include <QTemporaryDir>
#include <QtTest>

#include <cmath>

using namespace appname::shell;

namespace {

constexpr int kDocW = 160;
constexpr int kDocH = 64;
constexpr int kEdgeX = 80; // the mask's hard step
constexpr int kProbeY = 32;

// The feather this net drives. Chosen so that:
//   * sigma = 24/3 = 8 stays on the DIRECT kernel path (the box
//     approximation only engages above sigma 32), so the truncation
//     radius is exact rather than approximate;
//   * the correct reach (24 px) and the stale-reading reach (3*24 = 72
//     px) fall on OPPOSITE sides of the far probe at 40 px.
constexpr double kFeather = 24.0;
constexpr int kFarProbe = 40; // > 24, < 72 — the discriminator

int luma(const QImage &img, int x, int y)
{
    const QColor c = img.pixelColor(x - 0, y - 0);
    return c.red();
}

} // namespace

class TstMaskFeather : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void init();

    void featherIsServedAtTheCompositeDoorNotJustTheCommandDoor();
    void featherReachIsFeatherPxNotSigmaPx();
    void densityReachesTheEnvelopeAndChangesEngineTruth();
    void enabledAndInvertedChangeEngineTruthInOppositeSenses();
    void linkedRoundTripsButIsNotYetAComposite();
    void outOfRangeIsClampedByConstructionAndRefusedRaw();
    void mergeKeyIsScopedPerMaskAndProp();
    void sessionedDragCoalescesToOneHistoryStep();
    void oneMaskPerLayerIsTheEngineSRuleAndBoundsWhatThisNetCanSee();
    void sessionMergeKeyIsBuiltFromTheMaskIdOnTheJournaledEnvelope();
    void fi18LargeFeatherHasNoTypedRefusalYet();
    void heldOnI18nUntilCatalogRowsLand();

private:
    // Builds the fixture; returns the mask id.
    QString buildFixture();
    // A SECOND masked target: an empty raster layer plus its own mask.
    // Returns the new mask id (empty on failure) and records the owning
    // layer in m_layer2Id. Built per-case rather than in buildFixture()
    // deliberately — an extra layer in the shared fixture would put every
    // composite assertion in this net on a stack it was not written
    // against, and this net's discriminating power lives in those
    // composites. The layer is EMPTY (layer.add makes a transparent
    // raster), so it composites to nothing either way.
    QString addSecondMaskedLayer();
    QImage composite();
    QVariantMap maskRow(const QString &maskId);
    QVariantMap maskRowOf(const QString &layerId, const QString &maskId);

    CatalogQml *m_i18n = nullptr;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QString m_layerId;
    QString m_maskId;
    QString m_layer2Id;
};

void TstMaskFeather::initTestCase()
{
    m_i18n = new CatalogQml(Catalog::load(QStringLiteral("en")));
    m_controller = new DocumentController(&m_i18n->catalog());
    m_announcer = new Announcer(&m_i18n->catalog());
    m_controller->setAnnouncer(m_announcer);
}

void TstMaskFeather::init()
{
    QVERIFY(m_controller->newDocument(kDocW, kDocH,
                                      QStringLiteral("white")));
    m_layer2Id.clear();
    m_maskId = buildFixture();
    QVERIFY(!m_maskId.isEmpty());
}

QString TstMaskFeather::buildFixture()
{
    // A raster layer over the white background.
    QVariantMap add;
    add.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    const QString newLayer = m_controller->newUuid();
    add.insert(QStringLiteral("newLayerId"), newLayer);
    if (!m_controller->applyCommand(QStringLiteral("layer.add"), {}, add, {})
             .value(QStringLiteral("ok"))
             .toBool())
        return {};
    m_layerId = newLayer;

    // Fill it opaque BLACK, with NO selection active so the fill covers
    // the whole canvas (fill honors the selection — §13.1).
    QVariantMap color;
    color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
    color.insert(QStringLiteral("comps"), QVariantList{0.0, 0.0, 0.0, 0.0});
    QVariantMap fill;
    fill.insert(QStringLiteral("content"), QStringLiteral("color"));
    fill.insert(QStringLiteral("color"), color);
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), newLayer);
    if (!m_controller->applyCommand(QStringLiteral("fill"), target, fill, {})
             .value(QStringLiteral("ok"))
             .toBool())
        return {};

    // A rect selection over the left half, and a mask built FROM it: the
    // hard step at kEdgeX this whole net measures against.
    QVariantMap rect;
    rect.insert(QStringLiteral("x"), 0);
    rect.insert(QStringLiteral("y"), 0);
    rect.insert(QStringLiteral("w"), kEdgeX);
    rect.insert(QStringLiteral("h"), kDocH);
    QVariantMap sel;
    sel.insert(QStringLiteral("source"), QStringLiteral("rect"));
    sel.insert(QStringLiteral("rect"), rect);
    if (!m_controller->applyCommand(QStringLiteral("selection.set"), {}, sel,
                                    {})
             .value(QStringLiteral("ok"))
             .toBool())
        return {};

    QVariantMap maskAdd;
    maskAdd.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    maskAdd.insert(QStringLiteral("from"), QStringLiteral("selection"));
    const QString newMask = m_controller->newUuid();
    maskAdd.insert(QStringLiteral("newMaskId"), newMask);
    if (!m_controller
             ->applyCommand(QStringLiteral("mask.add"), target, maskAdd, {})
             .value(QStringLiteral("ok"))
             .toBool())
        return {};

    // Drop the selection: it must not gate the readbacks below.
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    return newMask;
}

QString TstMaskFeather::addSecondMaskedLayer()
{
    // A second raster layer. layer.add makes it EMPTY (fully
    // transparent), so it contributes nothing to the composite and the
    // step-edge fixture the rest of this net measures is untouched.
    QVariantMap add;
    add.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    const QString newLayer = m_controller->newUuid();
    add.insert(QStringLiteral("newLayerId"), newLayer);
    if (!m_controller->applyCommand(QStringLiteral("layer.add"), {}, add, {})
             .value(QStringLiteral("ok"))
             .toBool())
        return {};

    // Its own mask, from a full-canvas selection so the mask has real
    // coverage to carry props against (an all-zero mask would still take
    // the props, but a coverage-bearing one is the honest analogue of
    // the first).
    QVariantMap rect;
    rect.insert(QStringLiteral("x"), 0);
    rect.insert(QStringLiteral("y"), 0);
    rect.insert(QStringLiteral("w"), kDocW);
    rect.insert(QStringLiteral("h"), kDocH);
    QVariantMap sel;
    sel.insert(QStringLiteral("source"), QStringLiteral("rect"));
    sel.insert(QStringLiteral("rect"), rect);
    if (!m_controller->applyCommand(QStringLiteral("selection.set"), {}, sel,
                                    {})
             .value(QStringLiteral("ok"))
             .toBool())
        return {};

    QVariantMap target;
    target.insert(QStringLiteral("layerId"), newLayer);
    QVariantMap maskAdd;
    maskAdd.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    maskAdd.insert(QStringLiteral("from"), QStringLiteral("selection"));
    const QString newMask = m_controller->newUuid();
    maskAdd.insert(QStringLiteral("newMaskId"), newMask);
    if (!m_controller
             ->applyCommand(QStringLiteral("mask.add"), target, maskAdd, {})
             .value(QStringLiteral("ok"))
             .toBool())
        return {};
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    m_layer2Id = newLayer;
    return newMask;
}

QImage TstMaskFeather::composite()
{
    QImage img;
    if (!m_controller->readRegionImage(QRect(0, 0, kDocW, kDocH), &img))
        return {};
    return img;
}

QVariantMap TstMaskFeather::maskRowOf(const QString &layerId,
                                      const QString &maskId)
{
    const QVariantList masks = m_controller->masksOf(layerId);
    for (const QVariant &v : masks) {
        const QVariantMap m = v.toMap();
        if (m.value(QStringLiteral("maskId")).toString() == maskId)
            return m;
    }
    return {};
}

QVariantMap TstMaskFeather::maskRow(const QString &maskId)
{
    return maskRowOf(m_layerId, maskId);
}

// ★ C-11, THE ABSENT-READBACK RULE, MECHANISED. `maskRow` returns an
// EMPTY map when the info door reports no such mask block, and
// QVariantMap::value() on a missing key returns a default-constructed
// QVariant — which reads as 0.0 / false. So `QCOMPARE(row.value("density")
// .toDouble(), 0.0)` and `QCOMPARE(row.value("enabled").toBool(), false)`
// PASS on a mask block that is not there at all: the assertion cannot
// tell "the engine set it to the falsy value" from "the engine reported
// nothing". Every readback whose EXPECTED value is the falsy one must
// therefore assert PRESENCE first. Proven, not asserted: neutering
// maskRow() to `return {}` leaves the un-guarded comparisons green and
// reds every guarded one.
#define VERIFY_MASK_FIELD_PRESENT(row, field)                                  \
    QVERIFY2((row).contains(QStringLiteral(field)),                            \
             "the info door reported NO `" field "` on this mask block — an "  \
             "absent block reads as the falsy value and would satisfy the "    \
             "comparison below VACUOUSLY (false-green checklist C-11)")

// ★ C-11 ACROSS TYPES (fix round 3). The macro above is written for
// QVariantMap — but the trap has NOTHING to do with QVariantMap. It is
// "the container's NO-RESULT value and the assertion's EXPECTED value
// are the same bytes", and EVERY type with a falsy default has that
// shape: QJsonObject, QVariantMap, QString, QImage, numeric zero, an
// empty list. `buildMaskSetPropsParams` documents its no-result as an
// EMPTY QJsonObject, and QJsonObject::value() on a missing key returns
// QJsonValue::Undefined, whose toDouble() is 0.0 — so a clamp-to-zero
// expectation is satisfied by a builder that emitted NOTHING.
// MEASURED, not asserted: under MUT-C11D (`if (v < maskPropMinimum(prop))
// return {};` — drop instead of clamp) the PRE-FIX net held 14 passed /
// 0 failed, the clamp case itself among the passes.
#define VERIFY_JSON_NUMBER_PRESENT(obj, field)                                 \
    do {                                                                       \
        QVERIFY2((obj).contains(QLatin1String(field)),                         \
                 "buildMaskSetPropsParams emitted NO `" field "` — an EMPTY "  \
                 "object is its documented NO-RESULT value and reads 0.0 on "  \
                 "every missing key, so the comparison below would pass "      \
                 "VACUOUSLY on a builder that DROPPED the value instead of "   \
                 "clamping it (false-green checklist C-11)");                  \
        QVERIFY2((obj).value(QLatin1String(field)).isDouble(),                 \
                 "`" field "` reached the params object as a NON-NUMBER; "     \
                 "toDouble() would read 0.0 off it and satisfy a "             \
                 "clamp-to-minimum expectation vacuously");                    \
    } while (false)

// ---------------------------------------------------------------------
// 1. Feather actually feathers — AT THE COMPOSITE, discriminatingly.
// ---------------------------------------------------------------------
void TstMaskFeather::featherIsServedAtTheCompositeDoorNotJustTheCommandDoor()
{
    const QImage hard = composite();
    QVERIFY(!hard.isNull());

    // The fixture really is a HARD step before any feather: scanning the
    // probe row, every pixel is one of the two extremes. If this fails
    // the fixture is not a step edge and nothing below measures reach.
    int intermediatesBefore = 0;
    for (int x = 0; x < kDocW; ++x) {
        const int v = luma(hard, x, kProbeY);
        if (v > 8 && v < 247)
            ++intermediatesBefore;
    }
    QCOMPARE(intermediatesBefore, 0);
    QVERIFY2(luma(hard, kEdgeX - 1, kProbeY) < 8,
             "left of the step is not the masked (black) extreme");
    QVERIFY2(luma(hard, kEdgeX + 1, kProbeY) > 247,
             "right of the step is not the unmasked (white) extreme");

    // Now feather. THE COMMAND DOOR accepting is not the claim — the
    // claim is that the COMPOSITE changed, which is the door that
    // refused before this arm landed.
    const QVariantMap r = m_controller->setMaskProp(
        m_layerId, m_maskId, QStringLiteral("feather"), kFeather);
    QVERIFY2(r.value(QStringLiteral("ok")).toBool(),
             qPrintable(QStringLiteral("mask.set_props feather refused: %1 %2")
                            .arg(r.value(QStringLiteral("code")).toString(),
                                 r.value(QStringLiteral("message")).toString())));

    // The engine's OWN record of the property (info door, not our echo).
    QCOMPARE(maskRow(m_maskId).value(QStringLiteral("feather")).toDouble(),
             kFeather);

    const QImage soft = composite();
    QVERIFY(!soft.isNull());

    // A GRADED EDGE: strictly intermediate values must now exist, and
    // they must be MONOTONE across the step (a Gaussian of a step is
    // monotone — noise or a botched kernel would not be).
    int intermediates = 0;
    for (int x = 0; x < kDocW; ++x) {
        const int v = luma(soft, x, kProbeY);
        if (v > 8 && v < 247)
            ++intermediates;
    }
    QVERIFY2(intermediates >= 8,
             qPrintable(QStringLiteral("no graded edge — %1 intermediate "
                                       "values across the step; the feather "
                                       "did not reach the composite")
                            .arg(intermediates)));
    for (int x = kEdgeX - kFarProbe; x < kEdgeX + kFarProbe; ++x) {
        QVERIFY2(luma(soft, x, kProbeY) <= luma(soft, x + 1, kProbeY),
                 qPrintable(QStringLiteral("ramp not monotone at x=%1")
                                .arg(x)));
    }
}

// ---------------------------------------------------------------------
// 2. sigma = feather_px/3 — the reach, where a wrong constant shows.
// ---------------------------------------------------------------------
void TstMaskFeather::featherReachIsFeatherPxNotSigmaPx()
{
    const QImage hard = composite();
    QVERIFY(m_controller
                ->setMaskProp(m_layerId, m_maskId, QStringLiteral("feather"),
                              kFeather)
                .value(QStringLiteral("ok"))
                .toBool());
    const QImage soft = composite();
    QVERIFY(!hard.isNull() && !soft.isNull());

    // THE INVARIANT, as the ENGINE defines it (mask_eval.rs:129-158):
    //   ceil(3*sigma) == ceil(feather_px) == Rmask, with sigma = feather_px/3.
    // The §16.3 kernel is TRUNCATED at ceil(3*sigma), so beyond exactly
    // feather_px from the step the feathered value is not merely close to
    // the unfeathered one, it is EXACTLY it. That makes the far probe a
    // hard equality rather than a tolerance.
    //
    // Under the STALE reading (sigma = feather_px) the reach would be
    // 3*24 = 72 px, and kFarProbe at 40 px would MOVE. This is the whole
    // discriminating point of the case.
    QVERIFY2(luma(soft, kEdgeX - kFarProbe, kProbeY)
                 == luma(hard, kEdgeX - kFarProbe, kProbeY),
             qPrintable(
                 QStringLiteral("feather reached %1 px LEFT of the step; the "
                                "invariant caps reach at feather_px = %2. "
                                "A sigma = feather_px reading (reach 3x) is "
                                "the classic cause.")
                     .arg(kFarProbe)
                     .arg(kFeather)));
    QVERIFY2(luma(soft, kEdgeX + kFarProbe, kProbeY)
                 == luma(hard, kEdgeX + kFarProbe, kProbeY),
             qPrintable(QStringLiteral("feather reached %1 px RIGHT of the "
                                       "step; reach must cap at feather_px")
                            .arg(kFarProbe)));

    // ...and it genuinely reaches INSIDE that radius, so the case cannot
    // be satisfied by a feather that does nothing at all.
    const int near = kEdgeX - int(kFeather) / 2;
    QVERIFY2(luma(soft, near, kProbeY) != luma(hard, near, kProbeY),
             "the composite is unchanged well INSIDE the feather radius — "
             "the feather is not being applied");
}

// ---------------------------------------------------------------------
// 3. density
// ---------------------------------------------------------------------
void TstMaskFeather::densityReachesTheEnvelopeAndChangesEngineTruth()
{
    // §11.2: m' = 1 - density*(1 - m). At density 0 EVERY pixel resolves
    // to full coverage, so the masked-away right half must go black.
    QVERIFY(luma(composite(), kEdgeX + 20, kProbeY) > 247);
    QVERIFY(m_controller
                ->setMaskProp(m_layerId, m_maskId, QStringLiteral("density"),
                              0.0)
                .value(QStringLiteral("ok"))
                .toBool());
    // PRESENCE FIRST — 0.0 is the falsy readback, so the comparison alone
    // is vacuous on an absent mask block (C-11; see the macro above).
    const QVariantMap atZero = maskRow(m_maskId);
    VERIFY_MASK_FIELD_PRESENT(atZero, "density");
    QCOMPARE(atZero.value(QStringLiteral("density")).toDouble(), 0.0);
    QVERIFY2(luma(composite(), kEdgeX + 20, kProbeY) < 8,
             "density 0 did not lerp coverage to 1 at the composite");

    // Half density lands strictly BETWEEN the two, which no boolean
    // reading of the field could produce.
    QVERIFY(m_controller
                ->setMaskProp(m_layerId, m_maskId, QStringLiteral("density"),
                              0.5)
                .value(QStringLiteral("ok"))
                .toBool());
    const int mid = luma(composite(), kEdgeX + 20, kProbeY);
    QVERIFY2(mid > 8 && mid < 247,
             qPrintable(QStringLiteral("density 0.5 gave %1, not an "
                                       "intermediate coverage")
                            .arg(mid)));
}

// ---------------------------------------------------------------------
// 4. enabled / inverted — opposite senses, both at the composite.
// ---------------------------------------------------------------------
void TstMaskFeather::enabledAndInvertedChangeEngineTruthInOppositeSenses()
{
    // enabled=false: the mask drops out of the stack entirely, so the
    // black layer covers everything.
    QVERIFY(m_controller
                ->setMaskProp(m_layerId, m_maskId, QStringLiteral("enabled"),
                              false)
                .value(QStringLiteral("ok"))
                .toBool());
    // PRESENCE FIRST — `false` is the falsy readback (C-11).
    const QVariantMap disabled = maskRow(m_maskId);
    VERIFY_MASK_FIELD_PRESENT(disabled, "enabled");
    QCOMPARE(disabled.value(QStringLiteral("enabled")).toBool(), false);
    QVERIFY2(luma(composite(), kEdgeX + 20, kProbeY) < 8,
             "a DISABLED mask still masked the composite");
    QVERIFY2(luma(composite(), kEdgeX - 20, kProbeY) < 8,
             "left half changed when the mask was disabled");

    QVERIFY(m_controller
                ->setMaskProp(m_layerId, m_maskId, QStringLiteral("enabled"),
                              true)
                .value(QStringLiteral("ok"))
                .toBool());
    QVERIFY(luma(composite(), kEdgeX + 20, kProbeY) > 247);

    // inverted=true: coverage flips, so the two halves SWAP. Asserting
    // both sides is what separates "inverted" from "disabled" — a
    // sense-inversion defect that only cleared one side would survive a
    // one-sided check.
    QVERIFY(m_controller
                ->setMaskProp(m_layerId, m_maskId, QStringLiteral("inverted"),
                              true)
                .value(QStringLiteral("ok"))
                .toBool());
    QCOMPARE(maskRow(m_maskId).value(QStringLiteral("inverted")).toBool(),
             true);
    QVERIFY2(luma(composite(), kEdgeX - 20, kProbeY) > 247,
             "inverted: the previously MASKED-IN left half is still black");
    QVERIFY2(luma(composite(), kEdgeX + 20, kProbeY) < 8,
             "inverted: the previously MASKED-OUT right half is still white");
}

// ---------------------------------------------------------------------
// 5. linked — HONEST: it round-trips, it does not composite. Yet.
// ---------------------------------------------------------------------
void TstMaskFeather::linkedRoundTripsButIsNotYetAComposite()
{
    // `linked` "alters sample space" (§11.2) — which only has an
    // observable meaning once the owning layer carries a TRANSFORM, and
    // masks on transformed layers are §11.2 Sub-slice C, still refused
    // typed at this base. Derived first-hand rather than assumed: the
    // ONLY consumers of MaskNode::linked() in the tree are the two
    // reporting doors (cabi doc.rs:539, events.rs:100), persistence
    // (io/package.rs:268,282) and the DAG leaf hash
    // (engine/mod.rs:2031). `graph/mask_eval.rs` — the one routine all
    // three compositing doors resolve coverage through — never reads it.
    //
    // So this case asserts EXACTLY what is true: the property reaches the
    // engine and the engine reports it back changed. It deliberately does
    // NOT assert a pixel change, because there is none to assert, and a
    // case that pretended otherwise would be the fake-success this row's
    // census rule exists to prevent.
    QCOMPARE(maskRow(m_maskId).value(QStringLiteral("linked")).toBool(), true);
    const QImage before = composite();
    // C-11 ACROSS TYPES (fix round 3 sweep): QImage's no-result value is a
    // NULL image, and null == null. So `composite() == before` — an
    // assertion of NO CHANGE — is satisfied by a readback door that
    // returned nothing BOTH times. Positive control first.
    QVERIFY2(!before.isNull(),
             "the composite readback door returned a NULL image, which would "
             "make the no-change comparison below true VACUOUSLY");
    QVERIFY(m_controller
                ->setMaskProp(m_layerId, m_maskId, QStringLiteral("linked"),
                              false)
                .value(QStringLiteral("ok"))
                .toBool());
    // PRESENCE FIRST — `false` is the falsy readback (C-11).
    const QVariantMap unlinked = maskRow(m_maskId);
    VERIFY_MASK_FIELD_PRESENT(unlinked, "linked");
    QCOMPARE(unlinked.value(QStringLiteral("linked")).toBool(), false);
    QVERIFY2(composite() == before,
             "`linked` CHANGED the composite — mask_eval grew a consumer, so "
             "this case's honest-limit framing is stale: give linked a real "
             "pixel assertion and re-check the Sub-slice C boundary");
}

// ---------------------------------------------------------------------
// 6. Ranges: clamped by construction, and refused when sent raw.
// ---------------------------------------------------------------------
void TstMaskFeather::outOfRangeIsClampedByConstructionAndRefusedRaw()
{
    // The builder clamps to the SCHEMA's declared bounds...
    QCOMPARE(maskPropMinimum(MaskProp::Feather), 0.0);
    QCOMPARE(maskPropMaximum(MaskProp::Feather), 1000.0);
    QCOMPARE(maskPropMaximum(MaskProp::Density), 1.0);
    const QJsonObject over = buildMaskSetPropsParams(MaskProp::Density, 1.5);
    VERIFY_JSON_NUMBER_PRESENT(over, "density");
    QCOMPARE(over.value(QLatin1String("density")).toDouble(), 1.0);
    // ★ THE CLAMP-TO-MINIMUM SITE, and the one that needs the guard most:
    // its expected value (0.0) IS the missing-key readback of its own
    // container, so the comparison ALONE cannot tell "clamped to the
    // minimum" from "key absent / object empty" — and an empty object is
    // exactly what this builder returns when it declines. See the macro.
    const QJsonObject under = buildMaskSetPropsParams(MaskProp::Feather, -5.0);
    VERIFY_JSON_NUMBER_PRESENT(under, "feather");
    QCOMPARE(under.value(QLatin1String("feather")).toDouble(), 0.0);
    // Non-finite: the contract's inlined §0.3/§40.1 policy — NaN to the
    // field's default, +/-Inf to min/max, never a validation error. These
    // two need NO presence guard and it would be noise: their expected
    // values (1.0 and 1000.0) are NOT the missing-key readback, so an
    // absent key reds them on the comparison itself.
    QCOMPARE(buildMaskSetPropsParams(MaskProp::Density,
                                     std::numeric_limits<double>::quiet_NaN())
                 .value(QLatin1String("density"))
                 .toDouble(),
             1.0);
    QCOMPARE(buildMaskSetPropsParams(MaskProp::Feather,
                                     std::numeric_limits<double>::infinity())
                 .value(QLatin1String("feather"))
                 .toDouble(),
             1000.0);
    // A bool prop can never be emitted as a number, and vice versa.
    QVERIFY(buildMaskSetPropsParams(MaskProp::Enabled, 1.0).isEmpty());
    QVERIFY(buildMaskSetPropsParams(MaskProp::Feather, true).isEmpty());

    // ★ ...and now DRIVE it (C-7): everything above reads the builder's
    // SHAPE, which is not the door's BEHAVIOUR. A below-minimum value must
    // be CLAMPED AND SENT, never dropped. A builder that returned {} here
    // takes setMaskProp's `params.isEmpty()` arm (document_controller.cpp,
    // the client-refusal path) and comes back ok:false with an EMPTY code
    // — byte-for-byte the return a TYPO'd property name gets, with no
    // status surface, no history step and no engine call. That is the
    // USER-VISIBLE cost of the vacuity the guards above close, and the
    // history equality below separates "clamped and sent" (+1) from
    // "silently dropped" (+0).
    QVERIFY(m_controller
                ->setMaskProp(m_layerId, m_maskId, QStringLiteral("feather"),
                              12.0)
                .value(QStringLiteral("ok"))
                .toBool());
    // ★ ...and 12.0 LANDED. Without this readback the sentence below is
    // an ARGUMENT, not a measurement. VERIFY_MASK_FIELD_PRESENT closes
    // key-ABSENCE; it cannot separate "clamped to the minimum" from "the
    // field was never written, and 0.0 is its construction default" — and
    // this file pins 0.0 as exactly that default at the builder site above
    // (the `under` readback). A setMaskProp that answered ok:true while
    // writing nothing would leave the row at 0.0 across BOTH calls and
    // satisfy the clamp comparison anyway: the broken instrument's failure
    // mode coincides with its healthy answer. Reading 12.0 here is the
    // liveness floor that makes the 0.0 below a CHANGE rather than a
    // coincidence.
    const QVariantMap seededRow = maskRow(m_maskId);
    VERIFY_MASK_FIELD_PRESENT(seededRow, "feather");
    QCOMPARE(seededRow.value(QStringLiteral("feather")).toDouble(), 12.0);
    const int stepsBeforeClamp = m_controller->historyEntries().size();
    const QVariantMap clamped = m_controller->setMaskProp(
        m_layerId, m_maskId, QStringLiteral("feather"), -5.0);
    QVERIFY2(clamped.value(QStringLiteral("ok")).toBool(),
             qPrintable(QStringLiteral(
                            "a BELOW-MINIMUM feather was REFUSED instead of "
                            "clamped (code=%1) — the builder dropped the "
                            "value rather than binding it to the minimum, so "
                            "a slider undershooting its track would "
                            "client-refuse with no surface")
                            .arg(clamped.value(QStringLiteral("code"))
                                     .toString())));
    QCOMPARE(int(m_controller->historyEntries().size()), stepsBeforeClamp + 1);
    // 12.0 was READ BACK above, then below-minimum: the readback at 0.0 is
    // therefore a real CHANGE off a measured 12.0, not the field's
    // untouched construction default.
    const QVariantMap clampedRow = maskRow(m_maskId);
    VERIFY_MASK_FIELD_PRESENT(clampedRow, "feather");
    QCOMPARE(clampedRow.value(QStringLiteral("feather")).toDouble(), 0.0);

    // ...and the clamp is not the shell inventing a limit: the ENGINE
    // refuses the same value typed when it is sent raw past the builder,
    // on the existing status path with its own code.
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), m_layerId);
    target.insert(QStringLiteral("maskId"), m_maskId);
    QVariantMap raw;
    raw.insert(QStringLiteral("density"), 1.5);
    const QVariantMap r = m_controller->applyCommand(
        QStringLiteral("mask.set_props"), target, raw, {});
    QVERIFY2(!r.value(QStringLiteral("ok")).toBool(),
             "the engine ACCEPTED density 1.5 — the schema's maximum is 1");
    QVERIFY2(r.value(QStringLiteral("code")).toString().contains(
                 QStringLiteral("out_of_range")),
             qPrintable(QStringLiteral("expected a typed out_of_range; got %1")
                            .arg(r.value(QStringLiteral("code")).toString())));
    const QString status = m_controller->statusText();
    QVERIFY2(status.contains(QStringLiteral("out_of_range")),
             qPrintable(QStringLiteral("the typed refusal did not reach the "
                                       "status surface; status was: %1")
                            .arg(status)));
    QVERIFY2(!status.startsWith(QStringLiteral("ui.status.")),
             "the status line key-echoed instead of resolving its id");

    // An unknown property name never reaches the wire at all. The
    // refusal carries an EMPTY code because the id it wants is HELD and
    // unminted (see setMaskProp) — and empty-code-and-not-ok is still
    // unambiguous, because every ENGINE refusal carries a non-empty one.
    const QVariantMap bad = m_controller->setMaskProp(
        m_layerId, m_maskId, QStringLiteral("feathr"), 4.0);
    // C-11 ACROSS TYPES (fix round 3 sweep): BOTH halves below are falsy
    // expectations on the SAME map — `ok` false and `code` empty — so an
    // entirely EMPTY return satisfies the pair, and an empty map is what a
    // door that stopped returning anything would give. Presence first.
    QVERIFY2(bad.contains(QStringLiteral("ok")),
             "the client-side refusal returned a map with no `ok` at all — "
             "an empty map reads false/empty and would satisfy both "
             "assertions below vacuously");
    QVERIFY2(bad.contains(QStringLiteral("code")),
             "the client-side refusal returned no `code` KEY — absent is not "
             "the same claim as present-and-empty, which is what this case "
             "pins");
    QVERIFY2(!bad.value(QStringLiteral("ok")).toBool(),
             "an unknown mask property name was accepted");
    // WIRED 2026-09-07 (B-12), ON THIS ASSERTION'S OWN INSTRUCTION: it read
    // "if that id is now minted and cataloged, wire it here and drop it from
    // the held list". A-05 minted `ui.error.bad_argument` on 2026-09-05 in
    // all four catalogs, so the door emits it and this pins the id rather
    // than its absence. The `message` stays EMPTY by the house pattern —
    // the caller resolves wording from the id (applyCommand's
    // ui.error.no_document does the same).
    QCOMPARE(bad.value(QStringLiteral("code")).toString(),
             QStringLiteral("ui.error.bad_argument"));
    QVERIFY2(m_i18n->catalog().has(QStringLiteral("ui.error.bad_argument")),
             "the door emits an id the catalog does not carry — that is the "
             "key-echo class the reasonId door exists to prevent");
    // ...and it really did not reach the engine: history did not grow.
    const int steps = m_controller->historyEntries().size();
    // C-11 ACROSS TYPES (fix round 3 sweep): "did not grow" is an equality
    // against a number that is ZERO on a history door reporting an EMPTY
    // list, and 0 == 0 passes. The fixture alone (layer.add, fill,
    // selection.set, mask.add) plus this case's own clamp drive have put
    // real steps on the stack, so a positive control is available and this
    // is it.
    QVERIFY2(steps > 0,
             "the history door reports an EMPTY stack after the fixture and "
             "a committed clamp — the no-growth comparison below would then "
             "be 0 == 0 and could not fail");
    m_controller->setMaskProp(m_layerId, m_maskId, QStringLiteral("feathr"),
                              4.0);
    QCOMPARE(int(m_controller->historyEntries().size()), steps);
}

// ---------------------------------------------------------------------
// 7. The §10.5 property-drag session key.
// ---------------------------------------------------------------------
void TstMaskFeather::mergeKeyIsScopedPerMaskAndProp()
{
    // Contract: "{kind}:{targetId}:{sessionUuid}#{propPath}", targetId =
    // the MASK id, "merge key scoped per (maskId, prop)".
    const QString s = QStringLiteral("11111111-2222-4333-8444-555555555555");
    const QString feather =
        maskPropSessionMergeKey(QStringLiteral("m1"), s, MaskProp::Feather);
    QCOMPARE(feather,
             QStringLiteral("mask.set_props:m1:") + s
                 + QStringLiteral("#feather"));
    // Same session, DIFFERENT property => different key. If these
    // collided, a feather drag and a density drag inside one gesture
    // would coalesce into a single undo step and one of the two values
    // would become unreachable by undo.
    QVERIFY(maskPropSessionMergeKey(QStringLiteral("m1"), s,
                                    MaskProp::Density)
            != feather);
    // Same property, DIFFERENT mask => different key.
    QVERIFY(maskPropSessionMergeKey(QStringLiteral("m2"), s,
                                    MaskProp::Feather)
            != feather);
    // No session uuid => no key => omitted from the wire (one history
    // step per call: the keyboard/wheel-step case).
    QVERIFY(maskPropSessionMergeKey(QStringLiteral("m1"), QString(),
                                    MaskProp::Feather)
                .isEmpty());

    // Engine truth for the no-session case: each call is its own step.
    const int before = m_controller->historyEntries().size();
    QVERIFY(m_controller
                ->setMaskProp(m_layerId, m_maskId, QStringLiteral("feather"),
                              4.0)
                .value(QStringLiteral("ok"))
                .toBool());
    QVERIFY(m_controller
                ->setMaskProp(m_layerId, m_maskId, QStringLiteral("feather"),
                              6.0)
                .value(QStringLiteral("ok"))
                .toBool());
    QVERIFY2(int(m_controller->historyEntries().size()) >= before + 2,
             "two un-sessioned property calls did not grow history twice");

    // ★ EVERYTHING ABOVE IS SHAPE, NOT SESSION. These assertions call
    // the key builder DIRECTLY and then drive the engine UN-SESSIONED,
    // so they cannot see whether `setMaskProp` actually passes the key
    // it builds. The engine-truth half lives in the next case; do not
    // read this one as covering the session.
}

// ---------------------------------------------------------------------
// 7b. THE SESSION, DRIVEN — §10.5 coalescing asserted on ENGINE TRUTH.
// ---------------------------------------------------------------------
void TstMaskFeather::sessionedDragCoalescesToOneHistoryStep()
{
    // ★ WHY THIS CASE EXISTS (§5 fix round, MAJOR-1). The case above
    // asserts the merge key's STRING SHAPE and then drives the engine
    // with UN-SESSIONED calls — which is precisely the state a
    // regression that DROPPED the key would produce. So the shape
    // assertions alone let `setMaskProp` pass QString() as its mergeKey
    // and stay green; the reviewer's mutation proved exactly that, and
    // the net held at 11 passed / 0 failed with every envelope's session
    // key nulled.
    //
    // What that regression costs is not a wrong pixel — it is a wrecked
    // undo stack. One feather drag becomes N undo steps instead of 1,
    // and the user's single Cmd+Z walks back through the drag one
    // sample at a time. That is user-visible. Hence: drive the session,
    // and read the ENGINE's history, not the key.
    //
    // ★ CORRECTED (fix round 3, minor 1). This comment used to add "and
    // nothing else in this net reds on it". That is FALSE and was false
    // the moment round 2 landed the journal case:
    // `sessionMergeKeyIsBuiltFromTheMaskIdOnTheJournaledEnvelope` asserts
    // `log.contains(correct)`, which a DROPPED key fails outright. Two
    // cases own MUT-F, not one. The claim is dropped rather than re-scoped
    // — exclusivity was never what justified this case, and the same
    // over-claim appears (also corrected) at (d) below.
    const QString session =
        QStringLiteral("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee");
    // The three samples of one slider drag between pointer-down and
    // pointer-up (§10.5 session-boundary table). All schema-legal, all
    // distinct, and all inside 0..1000 so none is clamped.
    constexpr double kDrag[3] = { 3.0, 7.0, 11.0 };

    // (a) THE SESSION, DRIVEN.
    const int before = m_controller->historyEntries().size();
    for (const double v : kDrag) {
        QVERIFY2(m_controller
                     ->setMaskProp(m_layerId, m_maskId,
                                   QStringLiteral("feather"), v, session)
                     .value(QStringLiteral("ok"))
                     .toBool(),
                 "a sessioned mask property call was refused");
    }
    // EXACTLY one step, and the equality is deliberate. `>= before + 1`
    // would pass on three steps — the defect itself — and `<= before+3`
    // would pass on zero, i.e. on a fixture where nothing applied at
    // all. Only the equality separates COALESCED from both BROKEN and
    // INERT (false-green checklist C-2).
    QCOMPARE(int(m_controller->historyEntries().size()), before + 1);
    // ...and the one surviving step carries the LAST value of the drag,
    // read back through the info door's mask block. A coalesce that
    // dropped the final sample would be a different defect wearing the
    // same step count.
    QCOMPARE(maskRow(m_maskId).value(QStringLiteral("feather")).toDouble(),
             kDrag[2]);

    // (b) THE CONTRAST, same fixture, same three samples, no session:
    // history grows by THREE. This is what makes (a) an OBSERVATION
    // rather than an artifact of a fixture whose history cannot grow
    // (checklist C-1) — the discriminating power is measured here, in
    // the same case, not assumed.
    const int beforeUnsessioned = m_controller->historyEntries().size();
    for (const double v : kDrag) {
        QVERIFY(m_controller
                    ->setMaskProp(m_layerId, m_maskId,
                                  QStringLiteral("feather"), v + 1.0)
                    .value(QStringLiteral("ok"))
                    .toBool());
    }
    QCOMPARE(int(m_controller->historyEntries().size()),
             beforeUnsessioned + 3);

    // (c) The per-property scoping, now on ENGINE truth rather than on
    // string inequality: a DIFFERENT property inside the SAME session
    // carries a different #{propPath}, so it opens its OWN step instead
    // of coalescing into the feather drag — and its own two samples
    // then coalesce with each other.
    const int beforeDensity = m_controller->historyEntries().size();
    QVERIFY(m_controller
                ->setMaskProp(m_layerId, m_maskId,
                              QStringLiteral("density"), 0.5, session)
                .value(QStringLiteral("ok"))
                .toBool());
    QVERIFY(m_controller
                ->setMaskProp(m_layerId, m_maskId,
                              QStringLiteral("density"), 0.25, session)
                .value(QStringLiteral("ok"))
                .toBool());
    QCOMPARE(int(m_controller->historyEntries().size()), beforeDensity + 1);
    QCOMPARE(maskRow(m_maskId).value(QStringLiteral("density")).toDouble(),
             0.25);
    // The feather value the drag left behind is untouched by the density
    // session: the two sessions carry INDEPENDENT VALUES, not one
    // clobbering the other.
    QCOMPARE(maskRow(m_maskId).value(QStringLiteral("feather")).toDouble(),
             kDrag[2] + 1.0);
    // ★ WHAT (c) DOES NOT SHOW, stated so nobody over-reads it. It does
    // NOT by itself prove the #{propPath} suffix is present: coalescing
    // merges with the IMMEDIATELY PRECEDING step, and (b) left three
    // un-sessioned feather steps in between, so a propPath-less key
    // would still have opened a new step here and (c) would still pass.
    // The propPath scoping is covered by the key-inequality assertions
    // in mergeKeyIsScopedPerMaskAndProp; what (c) adds on ENGINE truth
    // is that a second session coalesces on its own and leaves the
    // first session's committed value alone.

    // (d) TWO MASKS, ADJACENT, ONE SESSION — the key must carry the
    // TARGET, not merely the session and the prop.
    //
    // ★ WHAT THIS SEES, AND WHAT IT PROVABLY CANNOT (fix round 2, C-1
    // asked as a MUTATION rather than as a note). The reviewer's remedy
    // for the surviving wrong-id mutant was "add a second mask, drag
    // across both, assert history grows by TWO". Built here — and it
    // catches a real defect class: a merge key that DROPS the target
    // (a constant key, or "{kind}::{session}#{prop}") makes these two
    // adjacent drags share one key, so they coalesce and history grows
    // by ONE.
    //
    // ★ CORRECTED (fix round 3, minor 1). Round 2's record said "nothing
    // else in the net reds on that defect class". FALSE:
    // `mergeKeyIsScopedPerMaskAndProp` compares the built key against the
    // CONTRACT's format string spelled out inline, so a key that dropped
    // its target reds THERE first. What is true, and is the reason this
    // section earns its place, is narrower: that case calls the key
    // BUILDER directly, so it cannot see a target dropped at the CALL SITE
    // in `setMaskProp`; this one drives the door and can.
    //
    // ★ BUT IT DOES NOT CLOSE THE WRONG-ID MUTANT, and saying so is the
    // point of this comment. Substituting the LAYER id for the mask id
    // in the key still grows history by TWO here, because the engine
    // admits AT MOST ONE MASK PER LAYER (kinds.rs MaskAddKind: a
    // non-empty masks() list returns error.command.target_in_use;
    // kind:"vector" returns NotImplemented) and mask.set_props refuses
    // any (layerId, maskId) pair whose mask is not on that layer
    // (resolve_mask_target: `!node.masks().contains(&mask)` ->
    // bad_target). maskId and layerId are therefore in BIJECTION for
    // every command the engine will accept, so the two readings induce
    // the IDENTICAL equality relation on consecutive merge keys — and
    // coalescing consults nothing but that equality (engine/mod.rs:829).
    // No fixture of any mask count can make a history-step observation
    // distinguish them at this base. Pinned as engine truth by
    // oneMaskPerLayerIsTheEngineSRuleAndBoundsWhatThisNetCanSee; the
    // wrong-id mutant is closed instead on the JOURNALED ENVELOPE, in
    // sessionMergeKeyIsBuiltFromTheMaskIdOnTheJournaledEnvelope.
    const QString mask2 = addSecondMaskedLayer();
    QVERIFY2(!mask2.isEmpty(), "the second masked layer did not build");
    QVERIFY(!m_layer2Id.isEmpty() && m_layer2Id != m_layerId);
    const QString session2 =
        QStringLiteral("bbbbbbbb-cccc-4ddd-8eee-ffffffffffff");
    constexpr double kDragA[3] = { 2.0, 5.0, 9.0 };
    constexpr double kDragB[3] = { 13.0, 17.0, 21.0 };

    // The two drags are BACK-TO-BACK: no command of any kind between
    // them. That adjacency is load-bearing — coalescing only ever merges
    // with the IMMEDIATELY PRECEDING step, so an intervening step would
    // let a target-blind key open a new step anyway and this assertion
    // would pass on the defect it exists to catch.
    const int beforeTwoMask = m_controller->historyEntries().size();
    for (const double v : kDragA) {
        QVERIFY(m_controller
                    ->setMaskProp(m_layerId, m_maskId,
                                  QStringLiteral("feather"), v, session2)
                    .value(QStringLiteral("ok"))
                    .toBool());
    }
    for (const double v : kDragB) {
        QVERIFY(m_controller
                    ->setMaskProp(m_layer2Id, mask2,
                                  QStringLiteral("feather"), v, session2)
                    .value(QStringLiteral("ok"))
                    .toBool());
    }
    QCOMPARE(int(m_controller->historyEntries().size()), beforeTwoMask + 2);
    // ...and each mask kept its OWN drag's last value: two steps with the
    // values crossed or clobbered would be a different defect wearing the
    // same step count. Presence-guarded (C-11) — an absent mask block
    // would otherwise read 0.0.
    const QVariantMap rowA = maskRowOf(m_layerId, m_maskId);
    VERIFY_MASK_FIELD_PRESENT(rowA, "feather");
    QCOMPARE(rowA.value(QStringLiteral("feather")).toDouble(), kDragA[2]);
    const QVariantMap rowB = maskRowOf(m_layer2Id, mask2);
    VERIFY_MASK_FIELD_PRESENT(rowB, "feather");
    QCOMPARE(rowB.value(QStringLiteral("feather")).toDouble(), kDragB[2]);
}

// ---------------------------------------------------------------------
// 7c. THE BOUND ON 7b(d): one mask per layer, as ENGINE truth.
// ---------------------------------------------------------------------
void TstMaskFeather::oneMaskPerLayerIsTheEngineSRuleAndBoundsWhatThisNetCanSee()
{
    // This case exists so the limitation documented in 7b(d) is a
    // MACHINE-CHECKED fact rather than a claim in a comment, and so that
    // it REDS the day it stops being true. The moment a layer can carry
    // two masks, maskId and layerId stop being in bijection and the
    // wrong-id merge-key substitution becomes visible to a plain
    // history-step assertion — at which point 7b(d) should be extended to
    // two masks on ONE layer and the journal readback becomes belt and
    // braces rather than the only door.
    //
    // ★ THE BOUND OF THIS PIN, RECORDED RATHER THAN OVERSTATED (fix round
    // 3, minor 3). §11.2's one-raster-mask-per-layer slot rule is enforced
    // in EXACTLY ONE place in the tree — `MaskAddKind::apply`
    // (crates/appname-core/src/engine/kinds.rs, the `masks().is_empty()`
    // check that returns TargetInUse). `Document::attach_mask` /
    // `attach_mask_at` (graph/document.rs) enforce ROOT-immutability and
    // layer existence and then push onto `node.masks` UNCONDITIONALLY, and
    // `validate_tree` carries no mask-count rule. So the PACKAGE-LOAD path
    // — io/package.rs's mask rebuild loop, which calls `attach_mask` once
    // per mask entry in the file — can seat two masks on one layer without
    // any refusal, and this case would not see it. Derived first-hand; the
    // fix is ENGINE-SIDE and outside this lane's shell/** write fence, so
    // it is recorded here rather than patched. NOT pinnable from the shell
    // at this base either: `openDocument` is the only door in, the package
    // is a STORED zip whose document.v1.json would have to be rewritten
    // byte-wise to fabricate the two-mask case, and the engine will not
    // build such a document for us to save. Whoever closes it should move
    // the slot rule into `attach_mask_at` (or into `validate_tree`) so the
    // load door is inside the same tripwire as the command door.

    // (a) A second RASTER mask on the layer that already has one is
    // refused typed, on the engine's own status path.
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), m_layerId);
    QVariantMap second;
    second.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    second.insert(QStringLiteral("from"), QStringLiteral("reveal_all"));
    second.insert(QStringLiteral("newMaskId"), m_controller->newUuid());
    const QVariantMap r = m_controller->applyCommand(
        QStringLiteral("mask.add"), target, second, {});
    QVERIFY2(!r.value(QStringLiteral("ok")).toBool(),
             "a SECOND mask attached to one layer — the §11.2 slot rule "
             "changed. maskId and layerId are no longer in bijection, so "
             "the mask-scoping of the §10.5 merge key is now observable "
             "through history coalescing: extend the two-mask drag in "
             "sessionedDragCoalescesToOneHistoryStep to put both masks on "
             "ONE layer and assert history grows by TWO there");
    QVERIFY2(r.value(QStringLiteral("code")).toString().contains(
                 QStringLiteral("target_in_use")),
             qPrintable(QStringLiteral("expected a typed target_in_use; got "
                                       "%1")
                            .arg(r.value(QStringLiteral("code")).toString())));

    // (b) ...and the vector slot the contract's targetResolution note
    // mentions ("at most one raster + one vector mask per layer") is not
    // built at this base either, so it is not an escape hatch for a
    // two-mask-one-layer fixture.
    //
    // ★ THE CODE IS ASSERTED, NOT JUST THE REFUSAL (fix round 3, minor 2).
    // `!ok` alone is a TRIPWIRE THAT CANNOT RING WHEN ITS OWN PREMISE
    // EXPIRES: the `path` uuid below is FABRICATED — it names no entry in
    // the path arena — so the day vector masks land, this mask.add starts
    // being refused for an UNRESOLVABLE PATH instead of for
    // not-implemented, `!ok` stays true, and the case stays GREEN through
    // the exact event that breaks the bijection the whole wrong-id
    // argument rests on. Pinning the TYPED code closes that: today it is
    // `error.command.not_implemented` (kinds.rs MaskAddKind, ahead of the
    // §11.2 slot check, so it is what a layer WITH a mask returns too),
    // and any other code — bad_target on the fabricated path included —
    // reds and says what to do.
    QVariantMap vec;
    vec.insert(QStringLiteral("kind"), QStringLiteral("vector"));
    vec.insert(QStringLiteral("newMaskId"), m_controller->newUuid());
    vec.insert(QStringLiteral("path"), m_controller->newUuid());
    const QVariantMap rv = m_controller->applyCommand(
        QStringLiteral("mask.add"), target, vec, {});
    QVERIFY2(!rv.value(QStringLiteral("ok")).toBool(),
             "vector masks now build — the second-mask slot on ONE layer is "
             "open; see (a)'s message");
    QVERIFY2(rv.value(QStringLiteral("code")).toString()
                 == QStringLiteral("error.command.not_implemented"),
             qPrintable(QStringLiteral(
                            "the vector mask slot no longer refuses "
                            "NOT_IMPLEMENTED; it refused with `%1`. If that "
                            "is a path-resolution refusal, vector masks have "
                            "LANDED and this case stopped guarding the "
                            "bijection: rebuild it against a REAL arena path "
                            "and move the two-mask drag in "
                            "sessionedDragCoalescesToOneHistoryStep onto ONE "
                            "layer. See (a)'s message")
                            .arg(rv.value(QStringLiteral("code")).toString())));

    // (c) The ownership check that makes the bijection airtight: a
    // mask.set_props naming a mask that is not on the named layer is
    // refused, so the net cannot drive one mask under two layer ids
    // either.
    const QString mask2 = addSecondMaskedLayer();
    QVERIFY2(!mask2.isEmpty(), "the second masked layer did not build");
    QVariantMap crossed;
    crossed.insert(QStringLiteral("layerId"), m_layer2Id);
    crossed.insert(QStringLiteral("maskId"), m_maskId); // owned by m_layerId
    QVariantMap params;
    params.insert(QStringLiteral("feather"), 4.0);
    const QVariantMap rc = m_controller->applyCommand(
        QStringLiteral("mask.set_props"), crossed, params, {});
    QVERIFY2(!rc.value(QStringLiteral("ok")).toBool(),
             "the engine accepted a mask under a layer that does not own it "
             "— one mask can now be addressed under two layer ids, which "
             "breaks the bijection in the other direction and again makes "
             "the merge key's target observable through coalescing");
    QVERIFY2(rc.value(QStringLiteral("code")).toString().contains(
                 QStringLiteral("bad_target")),
             qPrintable(QStringLiteral("expected a typed bad_target; got %1")
                            .arg(rc.value(QStringLiteral("code")).toString())));
}

// ---------------------------------------------------------------------
// 7d. THE WRONG-ID MUTANT, CLOSED — on the JOURNALED envelope.
// ---------------------------------------------------------------------
void TstMaskFeather::sessionMergeKeyIsBuiltFromTheMaskIdOnTheJournaledEnvelope()
{
    // ★ WHY THIS DOOR. The contract's targetResolution note is explicit:
    // the merge key is "{kind}:{targetId}:{sessionUuid}#{propPath}" with
    // "targetId = the maskId". 7b(d) and the case above establish that
    // NO history-step observation can see which id `setMaskProp` actually
    // passed, because the two candidate ids are in bijection. The merge
    // key is not reported by the info door, the history door
    // (index/id/kind/timestamp/active) or the events door, so exactly one
    // engine-written artifact carries it: the §7.8 journal. `saveDocument`
    // binds the standalone autosave log, and every envelope the engine
    // COMMITS is appended to it as `env.to_json()` — the engine's own
    // re-serialization of the envelope IT parsed and accepted, not the
    // shell's echo of what it meant to send.
    //
    // The expected key is spelled out here from the CONTRACT's format
    // string against the fixture's own ids, NOT rebuilt with
    // maskPropSessionMergeKey — a test that built its expectation with
    // the same helper the door uses would move with it and could not see
    // a wrong ARGUMENT at the call site, which is precisely the mutant.
    QTemporaryDir dir;
    QVERIFY2(dir.isValid(), "no temporary directory for the journal readback");
    const QString pkg = dir.filePath(QStringLiteral("row6-journal.appname"));
    QVERIFY2(m_controller->saveDocument(pkg),
             qPrintable(QStringLiteral("save refused: %1")
                            .arg(m_controller->statusText())));
    const QString logPath = pkg + QStringLiteral(".autosave");
    QVERIFY2(QFileInfo::exists(logPath),
             "saveDocument did not bind the §7.8 autosave log — this case's "
             "readback door is gone (ABI 1.2 block gate, or the bind guard "
             "declined). Without it the merge key is unobservable and the "
             "mask-scoping claim would be string-only again");

    const QString session =
        QStringLiteral("cccccccc-dddd-4eee-8fff-000000000001");
    constexpr double kDrag[3] = { 2.0, 6.0, 10.0 };
    for (const double v : kDrag) {
        QVERIFY(m_controller
                    ->setMaskProp(m_layerId, m_maskId,
                                  QStringLiteral("feather"), v, session)
                    .value(QStringLiteral("ok"))
                    .toBool());
    }

    QFile f(logPath);
    QVERIFY2(f.open(QIODevice::ReadOnly), "the autosave log did not open");
    const QByteArray log = f.readAll();
    f.close();
    // POSITIVE CONTROL FIRST (checklist C-11 / C-2): an empty or
    // mask.set_props-less log would make every contains() below false and
    // the ABSENCE assertion vacuously true. Assert the door delivered
    // something before reading anything off it.
    QVERIFY2(!log.isEmpty(), "the journaled autosave log is EMPTY");
    QVERIFY2(log.contains(QByteArrayLiteral("mask.set_props")),
             "the journaled log carries no mask.set_props envelope at all — "
             "the sessioned calls above did not reach the journal, so the "
             "key assertions below would be reading nothing");

    const QByteArray correct = (QStringLiteral("mask.set_props:") + m_maskId
                                + QLatin1Char(':') + session
                                + QStringLiteral("#feather"))
                                   .toUtf8();
    const QByteArray wrongId = (QStringLiteral("mask.set_props:") + m_layerId
                                + QLatin1Char(':') + session
                                + QStringLiteral("#feather"))
                                   .toUtf8();
    QVERIFY2(log.contains(correct),
             qPrintable(QStringLiteral(
                            "the journaled envelope does NOT carry the "
                            "contract's merge key %1 — mask.set_props.yaml "
                            "specifies targetId = the maskId")
                            .arg(QString::fromUtf8(correct))));
    QVERIFY2(!log.contains(wrongId),
             qPrintable(QStringLiteral(
                            "the journaled envelope carries a merge key "
                            "scoped to the OWNING LAYER (%1) instead of the "
                            "mask — the contract's targetId is the maskId")
                            .arg(QString::fromUtf8(wrongId))));
    // The propPath suffix is part of the same call site: a key that
    // reached the journal without it would coalesce a feather drag and a
    // density drag inside one gesture into a single undo step.
    QVERIFY2(!log.contains((QStringLiteral("mask.set_props:") + m_maskId
                            + QLatin1Char(':') + session
                            + QStringLiteral("\""))
                               .toUtf8()),
             "a mask.set_props merge key reached the journal with NO "
             "#{propPath} suffix");
}

// ---------------------------------------------------------------------
// 8. FI-18 — THE ABSENCE, PINNED. Read the comment before changing this.
// ---------------------------------------------------------------------
void TstMaskFeather::fi18LargeFeatherHasNoTypedRefusalYet()
{
    // FI-18 (reports/V5.6-FOLD-IN-REGISTER.md:100, registered on main):
    // the feathered-mask canonical plane is the mask content INFLATED BY
    // Rmask on every side and rebuilt per composite, so it is UNBOUNDED
    // at the schema limit — doc.new admits 1e6 px/side and feather runs
    // to 1000. Measured by the engine lane at 4096^2 / feather 1000:
    // 567 MiB transient, 1.78 s per build.
    //
    // ★ THE DESK RULING IS NOT BUILT AT THIS BASE, AND THIS CASE SAYS SO
    // RATHER THAN PRETENDING OTHERWISE. FI-18 rules for "a TYPED REFUSAL
    // above an engine ceiling, no stricter than the existing
    // MAX_READBACK_PIXELS = 1<<26 precedent". That refusal DOES NOT
    // EXIST in pro/feather-b.
    //
    // ★ CORRECTED (§5 fix round, MAJOR-2.2) — "INEXPRESSIBLE" WAS
    // OVERSTATED, AND THE SCOPE MATTERS BECAUSE THIS REACHED A DESK
    // REGISTER RULING. It is true of the EVALUATOR and only there:
    // `mask_eval::feather_coverage` returns `Option<FeatheredCoverage>`
    // (mask_eval.rs:241) and `ResolvedMasks::resolve` returns a plain
    // `ResolvedMasks` (:324) — neither has an error arm, so no refusal
    // can be phrased at THAT door without changing those signatures.
    // But the COMMAND door is a different door and it ALREADY refuses
    // typed: contracts/commands/mask.set_props.yaml gives `feather`
    // `maximum: 1000` and registers `error.schema.out_of_range` in its
    // `errors:` list, and assertion (b) below drives that refusal
    // first-hand. So an FI-18 ceiling IS expressible at the command
    // door today — it is UNIMPLEMENTED there, not impossible. What is
    // structurally impossible at this base is only refusing from inside
    // the evaluator once the command has been accepted.
    //
    // The module's own doc says the failure mode is "aborts on
    // allocation rather than refusing typed", flagged and "deliberately
    // NOT fixed here". The only 1<<26 in the tree is cabi
    // composite.rs:51, which caps the requested READBACK RECT
    // (composite.rs:64) and never sees `feather`.
    //
    // So a UI cannot surface an FI-18 refusal today, and this net will
    // not fake one. What it CAN pin, and does:
    //   (a) the maximum the SCHEMA allows is accepted, not refused — so
    //       no shell surface may imply a smaller ceiling than 1000;
    //   (b) the slider is NOT unbounded — 1000 is a real limit and past
    //       it the engine refuses typed;
    //   (c) a TRIPWIRE for a VALUE-KEYED ceiling: if `feather` starts
    //       refusing at a value the schema allows — a lowered schema
    //       maximum, or any refusal keyed on the feather NUMBER — this
    //       case REDS, telling whoever landed it that the shell must
    //       now surface it.
    //
    // ★ CORRECTED (§5 fix round, MAJOR-2.1) — THE TRIPWIRE'S REACH,
    // STATED HONESTLY. (c) was previously written as though it caught
    // ANY FI-18 refusal. IT DOES NOT, and the gap is the LIKELIEST
    // shape of the fix: an AREA-KEYED ceiling would leave this case
    // GREEN FOREVER. The arithmetic, derived from the engine and not
    // assumed —
    //   * the canonical plane is `content.inflate(Rmask)` where
    //     `content = cov.bounds() UNION cov.tiles().occupied_bounds()`
    //     (mask_eval.rs feather_coverage) and Rmask =
    //     feather_reach_px(1000) = ceil(3 * 1000/3) = 1000 (:150);
    //   * `occupied_bounds` unions WHOLE tile rects (tile/map.rs:220-224)
    //     and TILE = 256 (tile/mod.rs:32), so THIS fixture's 80x64 mask
    //     rounds up to one 256x256 tile;
    //   * canonical = (256 + 2000)^2 = 2256 x 2256 = 5,089,536 px.
    //     (On the tighter written-rect reading, 2080 x 2064 =
    //     4,293,120 px — the conclusion is identical either way.)
    //   * the ceiling FI-18 names is MAX_READBACK_PIXELS = 1<<26 =
    //     67,108,864 px. This fixture sits ~13x under it.
    // The module doc's own leading candidate is "refuse typed above an
    // area threshold", and FI-18 requires any ceiling be "no stricter
    // than" 1<<26 — so no conforming area-keyed refusal can ever fire
    // on a fixture this small. Whoever lands FI-18 must therefore NOT
    // read a green here as "the shell is covered": an area-keyed
    // ceiling needs its own pin, on a fixture whose canonical plane
    // actually exceeds 1<<26.
    //
    // NOT closed here, deliberately. Building that fixture at this base
    // does not assert a refusal — no refusal exists — it just reaches
    // the allocation abort the module doc already flags, and at 1<<26 px
    // x 4 f32 lanes the source plane alone is ~1 GiB before the blur's
    // output. That is the same OOM this case has always refused to
    // drive (below), so the honest move is to NAME the gap rather than
    // to widen the claim past what the pin can see.
    //
    // DELIBERATELY NOT DONE: driving a canvas large enough to trip the
    // real allocation abort. That is an OOM, not an assertion — it would
    // take the runner down and prove only what the engine lane already
    // measured.
    const QVariantMap atMax = m_controller->setMaskProp(
        m_layerId, m_maskId, QStringLiteral("feather"), 1000.0);
    QVERIFY2(atMax.value(QStringLiteral("ok")).toBool(),
             qPrintable(QStringLiteral(
                            "feather 1000 (the SCHEMA MAXIMUM) was refused: "
                            "%1 %2 — if this is FI-18's typed ceiling "
                            "landing, the row-6 shell must now SURFACE it: "
                            "give the slider an engine-derived max and route "
                            "the refusal to ui.status.rejected")
                            .arg(atMax.value(QStringLiteral("code")).toString(),
                                 atMax.value(QStringLiteral("message"))
                                     .toString())));
    QCOMPARE(maskRow(m_maskId).value(QStringLiteral("feather")).toDouble(),
             1000.0);

    // (b) past the schema maximum the engine DOES refuse typed, and that
    // refusal reaches the existing status path with the engine's own
    // code. This is the honest bound a feather control may advertise.
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), m_layerId);
    target.insert(QStringLiteral("maskId"), m_maskId);
    QVariantMap raw;
    raw.insert(QStringLiteral("feather"), 1000.5);
    const QVariantMap over = m_controller->applyCommand(
        QStringLiteral("mask.set_props"), target, raw, {});
    QVERIFY2(!over.value(QStringLiteral("ok")).toBool(),
             "the engine accepted a feather ABOVE the schema maximum — the "
             "1000 bound the shell advertises is not real");
    QVERIFY(m_controller->statusText().contains(
        QStringLiteral("out_of_range")));
}

// ---------------------------------------------------------------------
// 9. THE i18n HOLD, PINNED MECHANICALLY (the row-7 pattern).
// ---------------------------------------------------------------------
void TstMaskFeather::heldOnI18nUntilCatalogRowsLand()
{
    // Row 6 ships NO property-panel chrome, because not one label the
    // controls need is cataloged. `Catalog::tr` KEY-ECHOES an uncataloged
    // id verbatim, so shipping the panel would print raw ids like
    // "ui.panel.mask.feather" at the user. This case records the hold as
    // a machine-checked fact and is DELIBERATELY a tripwire: when O-I18N
    // mints and catalogs these rows it REDS, and whoever lands them
    // learns the held controls are unblocked.
    //
    // NOT reused, deliberately: `ui.options.feather` and
    // `ui.dialog.modify.feather` DO exist, but they label the SELECTION
    // feather (the options bar and Select > Modify). Borrowing a
    // selection-tool id for a §11.2 mask property is id-squatting: the
    // two are different features that translate differently, and the
    // reuse would be invisible until a translator saw it. Holding is the
    // honest move; inventing ids is banned.
    const char *heldIds[] = {
        "ui.panel.mask.feather", "ui.panel.mask.density",
        "ui.panel.mask.enabled", "ui.panel.mask.inverted",
        "ui.panel.mask.linked",  "ui.panel.mask.title",
        // Not a caption: the client-side refusal code setMaskProp wants
        // for an unknown property name. Held on the same rule.
        "ui.error.bad_argument",
    };
    // ★★★★★ THE TRIPWIRE FIRED AND THIS IS THE OTHER SIDE OF IT (B-12,
    // 2026-09-07). The case above was written to RED "when O-I18N mints and
    // catalogs these rows … whoever lands them learns the held controls are
    // unblocked". A-05 minted all seven on 2026-09-05 in four locales, so
    // the hold IS over and the assertion inverts: every id must now be
    // PRESENT. Kept as a case rather than deleted, because the same list
    // still has to be machine-checked — a mint that later regressed would
    // otherwise take the panel's captions down silently.
    for (const char *id : heldIds) {
        QVERIFY2(m_i18n->catalog().has(QLatin1String(id)),
                 qPrintable(QStringLiteral("%1 is NOT cataloged — B-12 ships "
                                           "the mask property controls and "
                                           "this caption would key-echo a raw "
                                           "id at the user")
                                .arg(QLatin1String(id))));
    }
    // Corroboration that the hold is about the CATALOG and not the
    // engine: the kind is live (every case above proves it), the history
    // label IS cataloged, and so is the refusal surface the controls
    // would ride. Nothing but the control captions is missing.
    QVERIFY(m_i18n->catalog().has(QStringLiteral("history.mask.set_props")));
    QVERIFY(m_i18n->catalog().has(QStringLiteral("ui.status.rejected")));
}

int main(int argc, char *argv[])
{
    qputenv("QT_QPA_PLATFORM", "offscreen");
    QGuiApplication app(argc, argv);
    TstMaskFeather tc;
    return QTest::qExec(&tc, argc, argv);
}

#include "tst_mask_feather.moc"
