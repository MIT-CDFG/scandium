// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts/commands/vector.path_new.yaml (§20.1 VectorPath JSON —
// CubicSeg contiguity "p0 is the previous segment's p1", closure
// "closed => last.p1 == first.p0", no degenerate segments) — pins
// buildVectorPathJson's invariants BYTE-level through JSON serialization
// so the §20.1 bitwise-contiguity rule survives any refactor.

#include "envelope_builder.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QtTest>

using namespace appname::shell;

namespace {

QByteArray vecBytes(const QJsonValue &v)
{
    return QJsonDocument(v.toArray()).toJson(QJsonDocument::Compact);
}

QJsonArray segmentsOf(const QJsonObject &path)
{
    return path.value("subpaths").toArray().first().toObject()
        .value("segments").toArray();
}

PenAnchor plain(double x, double y)
{
    PenAnchor a;
    a.anchor = QPointF(x, y);
    a.handleOut = a.anchor;
    return a;
}

PenAnchor handled(double x, double y, double hx, double hy)
{
    PenAnchor a;
    a.anchor = QPointF(x, y);
    a.handleOut = QPointF(hx, hy);
    a.hasHandle = true;
    return a;
}

} // namespace

class TstVectorPath : public QObject {
    Q_OBJECT

private slots:
    void segmentCounts()
    {
        const QVector<PenAnchor> tri{plain(0, 0), plain(50, 0), plain(25, 40)};
        QCOMPARE(segmentsOf(buildVectorPathJson(tri, false)).size(), 2);
        QCOMPARE(segmentsOf(buildVectorPathJson(tri, true)).size(), 3);
        QCOMPARE(segmentsOf(buildVectorPathJson({plain(1, 1)}, false)).size(), 0);
    }

    // §20.1 bitwise contiguity: p0 of segment i serializes to EXACTLY the
    // bytes of p1 of segment i-1 (same doubles, duplicated verbatim).
    void contiguityBitwise()
    {
        const QVector<PenAnchor> a{handled(0.1, 0.2, 7.3, 9.9), plain(33.7, 21.1),
                                   handled(64.2, 5.5, 60.0, 1.25), plain(12.5, 48.75)};
        const QJsonArray segs = segmentsOf(buildVectorPathJson(a, true));
        QCOMPARE(segs.size(), 4);
        for (int i = 1; i < segs.size(); ++i)
            QCOMPARE(vecBytes(segs.at(i).toObject().value("p0")),
                     vecBytes(segs.at(i - 1).toObject().value("p1")));
    }

    void closureInvariant()
    {
        const QVector<PenAnchor> a{plain(3, 4), plain(90, 12), plain(45, 77)};
        const QJsonArray segs = segmentsOf(buildVectorPathJson(a, true));
        QCOMPARE(vecBytes(segs.last().toObject().value("p1")),
                 vecBytes(segs.first().toObject().value("p0")));
        const QJsonObject open = buildVectorPathJson(a, false);
        QCOMPARE(open.value("subpaths").toArray().first().toObject()
                     .value("closed").toBool(), false);
        QCOMPARE(open.value("fillRule").toString(), QStringLiteral("nonzero"));
    }

    // No-handle anchors: exact collinear 1/3-2/3 controls (straight,
    // non-degenerate per §20.1).
    void straightSegmentControls()
    {
        const QVector<PenAnchor> a{plain(0, 0), plain(30, 90)};
        const QJsonObject seg =
            segmentsOf(buildVectorPathJson(a, false)).first().toObject();
        QCOMPARE(seg.value("c1").toArray().at(0).toDouble(), 10.0);
        QCOMPARE(seg.value("c1").toArray().at(1).toDouble(), 30.0);
        QCOMPARE(seg.value("c2").toArray().at(0).toDouble(), 20.0);
        QCOMPARE(seg.value("c2").toArray().at(1).toDouble(), 60.0);
    }

    // handleOut drives c1; the DESTINATION's mirrored handleIn drives c2
    // (handleIn = 2*anchor - handleOut).
    void handleMirroring()
    {
        const QVector<PenAnchor> a{handled(10, 10, 20, 5), handled(50, 50, 60, 70)};
        const QJsonObject seg =
            segmentsOf(buildVectorPathJson(a, false)).first().toObject();
        QCOMPARE(seg.value("c1").toArray().at(0).toDouble(), 20.0);
        QCOMPARE(seg.value("c1").toArray().at(1).toDouble(), 5.0);
        QCOMPARE(seg.value("c2").toArray().at(0).toDouble(), 40.0); // 2*50-60
        QCOMPARE(seg.value("c2").toArray().at(1).toDouble(), 30.0); // 2*50-70
    }

    // §20.1 invariant 3 (review V-D1): natural duplicate-anchor gestures
    // must never emit a degenerate segment (stage 6 rejects the WHOLE
    // path). Handle-less consecutive duplicates collapse; a trailing
    // duplicate of the first anchor on a closed path drops in favor of
    // the closure segment; handled same-point anchors survive (valid
    // loop, not all four points coincident).
    void duplicateAnchorsNeverDegenerate()
    {
        // Two clicks on one snap target inside a longer session.
        const QVector<PenAnchor> dup{plain(0, 0), plain(40, 0), plain(40, 0),
                                     plain(40, 30)};
        const QJsonArray segs = segmentsOf(buildVectorPathJson(dup, false));
        QCOMPARE(segs.size(), 2);
        // Double-click-to-close: the click pair appends a duplicate of the
        // last anchor, then commit(closed) — and here the duplicate also
        // equals the FIRST anchor (closing on the start point).
        const QVector<PenAnchor> close{plain(0, 0), plain(50, 0), plain(25, 40),
                                       plain(0, 0)};
        const QJsonArray cs = segmentsOf(buildVectorPathJson(close, true));
        QCOMPARE(cs.size(), 3); // A->B, B->C, closure C->A
        for (const QJsonValue &sv : cs) {
            const QJsonObject seg = sv.toObject();
            QVERIFY(vecBytes(seg.value("p0")) != vecBytes(seg.value("p1"))
                    || vecBytes(seg.value("c1")) != vecBytes(seg.value("p0"))
                    || vecBytes(seg.value("c2")) != vecBytes(seg.value("p0")));
        }
        // Handled anchor at the same point: kept (valid tiny loop).
        const QVector<PenAnchor> loop{plain(10, 10), handled(10, 10, 30, 5)};
        QCOMPARE(segmentsOf(buildVectorPathJson(loop, false)).size(), 1);
        // Session that dedupes below drawability: zero segments.
        const QVector<PenAnchor> collapsed{plain(5, 5), plain(5, 5)};
        QCOMPARE(segmentsOf(buildVectorPathJson(collapsed, false)).size(), 0);
    }
};

QTEST_GUILESS_MAIN(TstVectorPath)
#include "tst_vector_path.moc"
