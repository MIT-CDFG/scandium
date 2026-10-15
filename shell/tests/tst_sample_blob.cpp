// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: the appname.samples.v1 byte contract (authority:
// crates/appname-tools/src/brush/sample.rs). The blob the shell builds
// must be BIT-EXACT vs the documented layout: the golden below is
// hand-computed from the layout table, not derived from the encoder.

#include "sample_blob.h"

#include <QtTest>

using namespace appname::shell;

class TstSampleBlob : public QObject {
    Q_OBJECT

private slots:
    void layoutConstantsPinned()
    {
        QCOMPARE(int(sizeof(InputSample)), 48);
        QCOMPARE(kSampleBlobHeaderBytes, 16);
        QCOMPARE(kSampleRecordBytes, 48);
        QCOMPARE(kSampleBlobVersion, quint16(1));
        QCOMPARE(kSampleBlobMaxCount, quint32(1048576));
    }

    void goldenSingleSampleBlobIsBitExact()
    {
        // One record, every field a hand-chosen exactly-representable
        // value; expected bytes written out from the layout table by hand.
        InputSample s;
        s.x = 1.5;        // f64 0x3FF8000000000000
        s.y = -2.0;       // f64 0xC000000000000000
        s.pressure = 0.5f; // f32 0x3F000000
        s.tiltX = 0.25f;   // f32 0x3E800000
        s.tiltY = -0.25f;  // f32 0xBE800000
        s.rotation = 1.0f; // f32 0x3F800000
        s.velocity = 3.0f; // f32 0x40400000
        s.t = 0.0;         // f64 0x0000000000000000

        const auto blob = encodeSamplesBlob({s});
        QVERIFY(blob.has_value());

        const unsigned char expected[64] = {
            // header: magic "APNSAMPL", version 1 LE, flags 0, count 1 LE
            0x41, 0x50, 0x4E, 0x53, 0x41, 0x4D, 0x50, 0x4C,
            0x01, 0x00,
            0x00, 0x00,
            0x01, 0x00, 0x00, 0x00,
            // x = 1.5 (f64 LE)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF8, 0x3F,
            // y = -2.0 (f64 LE)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0,
            // pressure 0.5 (f32 LE)
            0x00, 0x00, 0x00, 0x3F,
            // tilt_x 0.25 (f32 LE)
            0x00, 0x00, 0x80, 0x3E,
            // tilt_y -0.25 (f32 LE)
            0x00, 0x00, 0x80, 0xBE,
            // rotation 1.0 (f32 LE)
            0x00, 0x00, 0x80, 0x3F,
            // velocity 3.0 (f32 LE)
            0x00, 0x00, 0x40, 0x40,
            // reserved (canonical zero)
            0x00, 0x00, 0x00, 0x00,
            // t = 0.0 (f64 LE)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        };
        QCOMPARE(blob->size(), qsizetype(64));
        QCOMPARE(*blob,
                 QByteArray(reinterpret_cast<const char *>(expected), sizeof(expected)));
    }

    void headerFieldsAndRecordStrideForMultiSample()
    {
        QVector<InputSample> samples;
        appendNormalizedSample(samples, 0.0, 0.0, 0.5, 0.0);
        appendNormalizedSample(samples, 10.0, 5.0, 0.75, 0.008);
        appendNormalizedSample(samples, 20.0, 10.0, 1.0, 0.016);
        const auto blob = encodeSamplesBlob(samples);
        QVERIFY(blob.has_value());
        QCOMPARE(blob->size(), qsizetype(16 + 3 * 48));
        QCOMPARE(blob->left(8), QByteArray("APNSAMPL"));
        QCOMPARE(quint8(blob->at(12)), quint8(3)); // count LE low byte
        QCOMPARE(quint8(blob->at(13)), quint8(0));
        // reserved hole of record 1 (offset 16 + 48 + 36) is zero
        for (int i = 0; i < 4; ++i)
            QCOMPARE(quint8(blob->at(16 + 48 + 36 + i)), quint8(0));
    }

    void validationRejectsDocumentedCases()
    {
        QVERIFY(!validateSamples({}).valid); // count zero

        InputSample bad;
        bad.pressure = 1.5f; // out of 0..=1
        QVERIFY(!validateSamples({bad}).valid);

        InputSample tilt;
        tilt.tiltX = -1.01f;
        QVERIFY(!validateSamples({tilt}).valid);

        InputSample vel;
        vel.velocity = -0.1f;
        QVERIFY(!validateSamples({vel}).valid);

        InputSample nan;
        nan.x = std::nan("");
        QVERIFY(!validateSamples({nan}).valid);

        InputSample a, b;
        a.t = 0.5;
        b.t = 0.5; // tie: t must strictly increase
        QVERIFY(!validateSamples({a, b}).valid);
        QVERIFY(!encodeSamplesBlob({a, b}).has_value());

        InputSample negT;
        negT.t = -0.5;
        QVERIFY(!validateSamples({negT}).valid);
    }

    void normalizationEnforcesUpstreamDuties()
    {
        QVector<InputSample> samples;
        appendNormalizedSample(samples, 0.0, 0.0, 2.0, 0.0);   // pressure clamps to 1
        appendNormalizedSample(samples, 3.0, 4.0, -1.0, 0.0);  // tie: t nudged, pressure clamps to 0
        QCOMPARE(samples.size(), 2);
        QCOMPARE(samples[0].pressure, 1.0f);
        QCOMPARE(samples[1].pressure, 0.0f);
        QVERIFY(samples[1].t > samples[0].t);
        // velocity derived: 5 px over the nudged 0.0001 s
        QVERIFY(samples[1].velocity > 0.0f);
        QVERIFY(validateSamples(samples).valid);
    }
};

QTEST_GUILESS_MAIN(TstSampleBlob)
#include "tst_sample_blob.moc"
