// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §15.1 (normalized input samples: field ranges, finiteness,
// strictly-increasing t, tie handling at normalization time) and §9.2
// (48-byte record mirror); byte-format authority
// crates/appname-tools/src/brush/sample.rs (appname.samples.v1: 16-byte
// header + 48-byte records, reserved hole canonical zero, count
// 1..=1_048_576). TESTER-added adversarial coverage beyond
// tst_sample_blob.cpp: per-field non-finite sweeps, boundary accepts,
// -0.0 canonicalization, pre- vs post-normalization asymmetry, long
// identical-t runs, float-precision limits of the +0.0001 nudge,
// f32 velocity overflow, count boundaries at encode time, and byte
// layout at record 0 / record N-1 across every reserved hole.
//
// Two cases (large-t nudge, velocity narrowing) began life as XFAIL
// witnesses of tester-found defects; both were fixed in the same wave's
// builder round and the cases now assert the documented duty directly as
// regression witnesses.

#include "sample_blob.h"

#include <QtTest>

#include <cmath>
#include <cstring>
#include <limits>

using namespace appname::shell;

namespace {

InputSample goodAt(double t)
{
    InputSample s;
    s.x = 1.0;
    s.y = 2.0;
    s.pressure = 0.5f;
    s.t = t;
    return s;
}

double f64FromLe(const QByteArray &blob, int offset)
{
    quint64 bits = 0;
    for (int i = 7; i >= 0; --i)
        bits = (bits << 8) | quint8(blob.at(offset + i));
    double v = 0.0;
    std::memcpy(&v, &bits, sizeof(v));
    return v;
}

} // namespace

class TstSampleBlobAdversarial : public QObject {
    Q_OBJECT

private slots:
    // §15.1 finiteness: every field, every non-finite flavor, rejects at
    // validation and encode returns nullopt (builder covered NaN x only).
    void nonFiniteRejectsEveryFieldEveryFlavor_data()
    {
        QTest::addColumn<int>("field");
        QTest::addColumn<int>("flavor"); // 0 NaN, 1 +Inf, 2 -Inf
        // BOUNDS COME FROM THE CONTAINERS, NOT FROM LITERALS. The previous form
        // was `f < 8` beside an 8-element array and `v < 3` beside a 3-element
        // one: add a ninth field and this generator silently keeps testing
        // eight, with every row still green. Latent rather than wrong -- 8 == 8
        // today -- which is exactly why it is worth binding before it is not.
        // QStringList over std::size deliberately: std::size has ZERO precedent
        // in shell/tests or shell/src, QStringList has 30 files of it.
        const QStringList fields = {"x", "y", "pressure", "tiltX", "tiltY",
                                    "rotation", "velocity", "t"};
        const QStringList flavors = {"nan", "+inf", "-inf"};
        for (int f = 0; f < fields.size(); ++f)
            for (int v = 0; v < flavors.size(); ++v)
                QTest::addRow("%s_%s", qPrintable(fields[f]),
                              qPrintable(flavors[v])) << f << v;
    }

    void nonFiniteRejectsEveryFieldEveryFlavor()
    {
        QFETCH(int, field);
        QFETCH(int, flavor);
        const double d64 = flavor == 0 ? std::nan("")
            : flavor == 1              ? std::numeric_limits<double>::infinity()
                                       : -std::numeric_limits<double>::infinity();
        const float d32 = static_cast<float>(d64);

        InputSample s = goodAt(0.0);
        switch (field) {
        case 0: s.x = d64; break;
        case 1: s.y = d64; break;
        case 2: s.pressure = d32; break;
        case 3: s.tiltX = d32; break;
        case 4: s.tiltY = d32; break;
        case 5: s.rotation = d32; break;
        case 6: s.velocity = d32; break;
        case 7: s.t = d64; break;
        }
        const auto verdict = validateSamples({s});
        QVERIFY(!verdict.valid);
        QVERIFY(!verdict.detail.isEmpty()); // diagnostics carry a detail
        QVERIFY(!encodeSamplesBlob({s}).has_value());
    }

    // §15.1 range boundaries are inclusive: the exact endpoints validate;
    // one f32 ulp beyond rejects. rotation is documented finite-unbounded.
    void inclusiveBoundariesAcceptOneUlpBeyondRejects()
    {
        InputSample lo = goodAt(0.0);
        lo.pressure = 0.0f;
        lo.tiltX = -1.0f;
        lo.tiltY = -1.0f;
        lo.rotation = -1.0e30f; // finite, unbounded by contract
        lo.velocity = 0.0f;
        InputSample hi = goodAt(0.5);
        hi.pressure = 1.0f;
        hi.tiltX = 1.0f;
        hi.tiltY = 1.0f;
        hi.rotation = 1.0e30f;
        hi.velocity = std::numeric_limits<float>::max();
        QVERIFY(validateSamples({lo, hi}).valid);
        QVERIFY(encodeSamplesBlob({lo, hi}).has_value());

        InputSample p = goodAt(0.0);
        p.pressure = std::nextafter(1.0f, 2.0f);
        QVERIFY(!validateSamples({p}).valid);
        p = goodAt(0.0);
        p.pressure = std::nextafter(0.0f, -1.0f);
        QVERIFY(!validateSamples({p}).valid);
        p = goodAt(0.0);
        p.tiltX = std::nextafter(1.0f, 2.0f);
        QVERIFY(!validateSamples({p}).valid);
        p = goodAt(0.0);
        p.tiltY = std::nextafter(-1.0f, -2.0f);
        QVERIFY(!validateSamples({p}).valid);
        p = goodAt(0.0);
        p.velocity = std::nextafter(0.0f, -1.0f);
        QVERIFY(!validateSamples({p}).valid);
    }

    // -0.0 passes every `< 0.0` gate (IEEE -0.0 == 0.0) yet encodes with
    // the sign bit set — parity with the Rust authority, which accepts it
    // identically. Pinned: two byte-different encodings of semantically
    // identical streams exist (content-addressing wrinkle, see report).
    void negativeZeroAcceptedAndEncodesSignBit()
    {
        InputSample s = goodAt(0.0);
        s.t = -0.0;
        s.x = -0.0;
        s.velocity = -0.0f;
        QVERIFY(validateSamples({s}).valid);
        const auto blob = encodeSamplesBlob({s});
        QVERIFY(blob.has_value());
        // x at record offset +0 (blob offset 16): -0.0 f64 LE = 00..00 80.
        QCOMPARE(quint8(blob->at(16 + 7)), quint8(0x80));
        // t at record offset +40: sign byte is the last of the 8.
        QCOMPARE(quint8(blob->at(16 + 40 + 7)), quint8(0x80));
        // velocity -0.0 f32 at +32: high byte 0x80.
        QCOMPARE(quint8(blob->at(16 + 32 + 3)), quint8(0x80));
    }

    // Pre- vs post-normalization asymmetry: raw out-of-range pressure
    // rejects; the same value through appendNormalizedSample clamps.
    // Non-finite x/y/pressure/t all have documented normalization
    // substitutes (0.0 / 1.0 / 0.0).
    void normalizationClampsWhatValidationRejects()
    {
        InputSample raw = goodAt(0.0);
        raw.pressure = 2.0f;
        QVERIFY(!validateSamples({raw}).valid);
        raw.pressure = -1.0f;
        QVERIFY(!validateSamples({raw}).valid);

        QVector<InputSample> norm;
        appendNormalizedSample(norm, 0.0, 0.0, 2.0, 0.0);
        appendNormalizedSample(norm, 1.0, 1.0, -1.0, 0.008);
        QCOMPARE(norm[0].pressure, 1.0f);
        QCOMPARE(norm[1].pressure, 0.0f);
        QVERIFY(validateSamples(norm).valid);

        QVector<InputSample> nf;
        const double inf = std::numeric_limits<double>::infinity();
        appendNormalizedSample(nf, std::nan(""), inf, std::nan(""), std::nan(""));
        QCOMPARE(nf.size(), 1);
        QCOMPARE(nf[0].x, 0.0);       // non-finite x -> 0.0
        QCOMPARE(nf[0].y, 0.0);       // non-finite y -> 0.0
        QCOMPARE(nf[0].pressure, 1.0f); // non-finite pressure -> 1.0
        QCOMPARE(nf[0].t, 0.0);       // non-finite t -> 0.0
        appendNormalizedSample(nf, 3.0, 4.0, 0.5, -inf); // -inf t -> 0.0, tied -> nudged
        QCOMPARE(nf[1].t, 0.0001);
        QVERIFY(validateSamples(nf).valid);

        // Negative t clamps to 0 pre-tie-check.
        QVector<InputSample> neg;
        appendNormalizedSample(neg, 0.0, 0.0, 0.5, -5.0);
        QCOMPARE(neg[0].t, 0.0);
        QVERIFY(validateSamples(neg).valid);
    }

    // The +0.0001 nudge rule under a long run of identical timestamps:
    // monotonicity must hold for every consecutive pair, velocities stay
    // finite and non-negative, and the result validates + encodes.
    void manyIdenticalTValuesStayStrictlyMonotonic()
    {
        QVector<InputSample> samples;
        const int n = 10000;
        for (int i = 0; i < n; ++i)
            appendNormalizedSample(samples, i * 0.5, 0.0, 0.5, 0.25); // t identical every time
        QCOMPARE(samples.size(), n);
        for (int i = 1; i < n; ++i) {
            QVERIFY2(samples[i].t > samples[i - 1].t,
                     qPrintable(QStringLiteral("tie run broke at %1").arg(i)));
            QVERIFY(std::isfinite(samples[i].velocity));
            QVERIFY(samples[i].velocity >= 0.0f);
        }
        QVERIFY(validateSamples(samples).valid);
        const auto blob = encodeSamplesBlob(samples);
        QVERIFY(blob.has_value());
        QCOMPARE(blob->size(), qsizetype(16 + qsizetype(n) * 48));
        // Derived velocity for the nudged ties: 0.5 px over 0.0001 s.
        QCOMPARE(samples[1].velocity, static_cast<float>(0.5 / 0.0001));
    }

    // Float-precision limit of the nudge: at prev.t >= 2^40 s the +0.0001
    // is below half an ulp, prev.t + 0.0001 would round back to prev.t —
    // the function's documented duty ("the shell never hands the engine
    // bytes it would reject") requires strictness to survive any
    // magnitude a QML caller can forward (e.g. an epoch-milliseconds
    // timestamp ~1.7e12 > 2^40). Was tester DEFECT #1, fixed via
    // nextafter escalation; this is now the regression witness.
    void nudgeMonotonicityAtLargeT()
    {
        // Fixed (tester defect 1): when +0.0001 is absorbed by rounding at
        // this magnitude, normalization escalates to nextafter — strict
        // monotonicity holds at any t, and the blob validates.
        const double big = 1099511627776.0; // 2^40
        QVector<InputSample> samples;
        appendNormalizedSample(samples, 0.0, 0.0, 0.5, big);
        appendNormalizedSample(samples, 1.0, 0.0, 0.5, big); // tie at large t
        QCOMPARE(samples.size(), 2);
        QVERIFY(samples[1].t > samples[0].t);
        QVERIFY(validateSamples(samples).valid);
    }

    // Velocity derivation with a huge finite coordinate jump: dist/dt is a
    // finite double (~1e304) that must be clamped into f32 range before
    // narrowing (out-of-range narrowing is UB and would fail the blob's
    // own validation). Was tester DEFECT #2, fixed via clamp-to-FLT_MAX;
    // this is now the regression witness.
    void velocityF32OverflowOnHugeJump()
    {
        // Fixed (tester defect 2): the double velocity is clamped into f32
        // range before narrowing — finite (FLT_MAX) and validator-clean.
        QVector<InputSample> samples;
        appendNormalizedSample(samples, 0.0, 0.0, 0.5, 0.0);
        appendNormalizedSample(samples, 1.0e300, 0.0, 0.5, 0.0); // tie -> dt 0.0001
        QCOMPARE(samples.size(), 2);
        QVERIFY(std::isfinite(samples[1].velocity));
        QCOMPARE(samples[1].velocity, std::numeric_limits<float>::max());
        QVERIFY(validateSamples(samples).valid);
    }

    // Zero-distance tie: velocity must be exactly 0 (0 px over nudged dt),
    // and the first sample's derived velocity is 0 by definition.
    void velocityZeroDistanceAndFirstSample()
    {
        QVector<InputSample> samples;
        appendNormalizedSample(samples, 7.0, 7.0, 0.5, 0.0);
        QCOMPARE(samples[0].velocity, 0.0f);
        appendNormalizedSample(samples, 7.0, 7.0, 0.5, 0.0); // same point, tied t
        QCOMPARE(samples[1].velocity, 0.0f);
        QCOMPARE(samples[1].t, 0.0001);
        // Known derivation: 3-4-5 triangle over exactly 0.01 s = 500 px/s.
        appendNormalizedSample(samples, 10.0, 11.0, 0.5, 0.0101);
        QCOMPARE(samples[2].velocity, 500.0f);
        QVERIFY(validateSamples(samples).valid);
    }

    // t regression (going backwards) is rejected raw but treated exactly
    // like a tie by normalization: nudged to prev.t + 0.0001.
    void tRegressionRejectedRawNudgedNormalized()
    {
        QVERIFY(!validateSamples({goodAt(1.0), goodAt(0.5)}).valid);
        QVERIFY(!encodeSamplesBlob({goodAt(1.0), goodAt(0.5)}).has_value());

        QVector<InputSample> norm;
        appendNormalizedSample(norm, 0.0, 0.0, 0.5, 1.0);
        appendNormalizedSample(norm, 0.0, 0.0, 0.5, 0.25); // regression
        QCOMPARE(norm[1].t, 1.0 + 0.0001);
        QVERIFY(validateSamples(norm).valid);
    }

    // Count boundaries at encode time: exactly 1_048_576 encodes (header
    // count field byte-exact), 1_048_577 rejects without encoding, and
    // appendNormalizedSample drops (not overflows) past the cap.
    // The vector is built by direct struct fill (cheap, ~48 MiB peak).
    void countBoundariesAtMax()
    {
        const int maxCount = int(kSampleBlobMaxCount); // 1'048'576
        QVector<InputSample> samples(maxCount);
        for (int i = 0; i < maxCount; ++i) {
            samples[i].pressure = 1.0f;
            samples[i].t = double(i) * 0.001;
        }
        QVERIFY(validateSamples(samples).valid);

        // The cap also gates the normalization append: a further append is
        // dropped, size stays at the max.
        appendNormalizedSample(samples, 0.0, 0.0, 0.5, 1.0e9);
        QCOMPARE(samples.size(), maxCount);

        const auto blob = encodeSamplesBlob(samples);
        QVERIFY(blob.has_value());
        QCOMPARE(blob->size(), qsizetype(16) + qsizetype(maxCount) * 48);
        // count field 12..16 LE: 1_048_576 = 0x00100000.
        QCOMPARE(quint8(blob->at(12)), quint8(0x00));
        QCOMPARE(quint8(blob->at(13)), quint8(0x00));
        QCOMPARE(quint8(blob->at(14)), quint8(0x10));
        QCOMPARE(quint8(blob->at(15)), quint8(0x00));
        // Last record's t round-trips bit-exactly at the tail offset.
        const int lastT = 16 + (maxCount - 1) * 48 + 40;
        QCOMPARE(f64FromLe(*blob, lastT), double(maxCount - 1) * 0.001);

        // One over the cap: validation rejects, encode refuses.
        InputSample extra = goodAt(double(maxCount) * 0.001);
        samples.append(extra);
        QCOMPARE(samples.size(), maxCount + 1);
        const auto verdict = validateSamples(samples);
        QVERIFY(!verdict.valid);
        QVERIFY(verdict.detail.contains(QStringLiteral("1048576")));
        QVERIFY(!encodeSamplesBlob(samples).has_value());
    }

    // Byte layout at the boundaries of a multi-record blob: full 16-byte
    // header golden, record 0's first field and record N-1's last field at
    // hand-computed offsets, reserved hole zero across EVERY record, and
    // no trailing bytes (total = 16 + 48*N exactly).
    void recordBoundariesAndReservedHoles()
    {
        const int n = 5;
        QVector<InputSample> samples;
        for (int i = 0; i < n; ++i) {
            InputSample s;
            s.x = -1.5;              // f64 0xBFF8000000000000
            s.y = 0.0;
            s.pressure = 1.0f;
            s.rotation = 6.5f;       // finite > 2*pi is legal (normalized at eval)
            s.velocity = float(i);
            s.t = double(i);         // 4.0 at the last record: 0x4010000000000000
            samples.append(s);
        }
        const auto blob = encodeSamplesBlob(samples);
        QVERIFY(blob.has_value());
        QCOMPARE(blob->size(), qsizetype(16 + n * 48)); // exact: no trailing bytes

        const unsigned char header[16] = {
            0x41, 0x50, 0x4E, 0x53, 0x41, 0x4D, 0x50, 0x4C, // "APNSAMPL"
            0x01, 0x00,                                     // version 1 LE
            0x00, 0x00,                                     // flags 0
            0x05, 0x00, 0x00, 0x00,                         // count 5 LE
        };
        QCOMPARE(blob->left(16),
                 QByteArray(reinterpret_cast<const char *>(header), 16));

        // Record 0, field x at blob offset 16: -1.5 f64 LE.
        const unsigned char xLe[8] = {0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF8, 0xBF};
        QCOMPARE(blob->mid(16, 8),
                 QByteArray(reinterpret_cast<const char *>(xLe), 8));

        // Record N-1, field t at blob offset 16 + 4*48 + 40: 4.0 f64 LE —
        // also the final 8 bytes of the blob.
        const int lastTOff = 16 + (n - 1) * 48 + 40;
        QCOMPARE(lastTOff + 8, int(blob->size()));
        const unsigned char tLe[8] = {0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x40};
        QCOMPARE(blob->mid(lastTOff, 8),
                 QByteArray(reinterpret_cast<const char *>(tLe), 8));

        // Reserved hole (+36..40) canonical zero in EVERY record.
        for (int r = 0; r < n; ++r)
            for (int b = 0; b < 4; ++b)
                QCOMPARE(quint8(blob->at(16 + r * 48 + 36 + b)), quint8(0));
    }
};

QTEST_GUILESS_MAIN(TstSampleBlobAdversarial)
#include "tst_sample_blob_adversarial.moc"
