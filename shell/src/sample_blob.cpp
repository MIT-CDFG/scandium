// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §15.1 / §9.2; byte-format authority
// crates/appname-tools/src/brush/sample.rs (appname.samples.v1 v1 layout).

#include "sample_blob.h"

#include <QtEndian>

#include <algorithm>
#include <cmath>
#include <cstring>
#include <limits>

namespace appname::shell {

namespace {

bool allFinite(const InputSample &s)
{
    return std::isfinite(s.x) && std::isfinite(s.y) && std::isfinite(s.pressure)
        && std::isfinite(s.tiltX) && std::isfinite(s.tiltY) && std::isfinite(s.rotation)
        && std::isfinite(s.velocity) && std::isfinite(s.t);
}

template <typename T>
void appendLe(QByteArray &out, T value)
{
    T le{};
    if constexpr (sizeof(T) == 2) {
        le = qToLittleEndian(value);
        out.append(reinterpret_cast<const char *>(&le), 2);
    } else if constexpr (sizeof(T) == 4) {
        le = qToLittleEndian(value);
        out.append(reinterpret_cast<const char *>(&le), 4);
    } else {
        static_assert(sizeof(T) == 8);
        le = qToLittleEndian(value);
        out.append(reinterpret_cast<const char *>(&le), 8);
    }
}

void appendF32Le(QByteArray &out, float v)
{
    quint32 bits{};
    static_assert(sizeof(bits) == sizeof(v));
    std::memcpy(&bits, &v, sizeof(bits));
    appendLe<quint32>(out, bits);
}

void appendF64Le(QByteArray &out, double v)
{
    quint64 bits{};
    static_assert(sizeof(bits) == sizeof(v));
    std::memcpy(&bits, &v, sizeof(bits));
    appendLe<quint64>(out, bits);
}

} // namespace

SampleValidation validateSamples(const QVector<InputSample> &samples)
{
    if (samples.isEmpty())
        return {false, QStringLiteral("sample blob has zero samples")};
    if (static_cast<quint64>(samples.size()) > kSampleBlobMaxCount)
        return {false,
                QStringLiteral("sample count %1 over v1 maximum %2")
                    .arg(samples.size())
                    .arg(kSampleBlobMaxCount)};
    double prevT = -std::numeric_limits<double>::infinity();
    for (int i = 0; i < samples.size(); ++i) {
        const InputSample &s = samples.at(i);
        if (!allFinite(s))
            return {false, QStringLiteral("record %1: non-finite field").arg(i)};
        if (s.pressure < 0.0f || s.pressure > 1.0f)
            return {false, QStringLiteral("record %1: pressure out of 0..=1").arg(i)};
        if (s.tiltX < -1.0f || s.tiltX > 1.0f)
            return {false, QStringLiteral("record %1: tilt_x out of -1..=1").arg(i)};
        if (s.tiltY < -1.0f || s.tiltY > 1.0f)
            return {false, QStringLiteral("record %1: tilt_y out of -1..=1").arg(i)};
        if (s.velocity < 0.0f)
            return {false, QStringLiteral("record %1: velocity negative").arg(i)};
        if (s.t < 0.0)
            return {false, QStringLiteral("record %1: t negative").arg(i)};
        if (s.t <= prevT)
            return {false, QStringLiteral("record %1: t not strictly increasing").arg(i)};
        prevT = s.t;
    }
    return {true, QString()};
}

std::optional<QByteArray> encodeSamplesBlob(const QVector<InputSample> &samples)
{
    if (!validateSamples(samples).valid)
        return std::nullopt;
    QByteArray out;
    out.reserve(kSampleBlobHeaderBytes + kSampleRecordBytes * samples.size());
    out.append(kSampleBlobMagic, sizeof(kSampleBlobMagic));
    appendLe<quint16>(out, kSampleBlobVersion);
    appendLe<quint16>(out, 0); // flags: v1 defines none
    appendLe<quint32>(out, static_cast<quint32>(samples.size()));
    for (const InputSample &s : samples) {
        appendF64Le(out, s.x);
        appendF64Le(out, s.y);
        appendF32Le(out, s.pressure);
        appendF32Le(out, s.tiltX);
        appendF32Le(out, s.tiltY);
        appendF32Le(out, s.rotation);
        appendF32Le(out, s.velocity);
        appendLe<quint32>(out, 0); // reserved: the C struct's alignment hole, canonical zero
        appendF64Le(out, s.t);
    }
    return out;
}

void appendNormalizedSample(QVector<InputSample> &samples,
                            double x,
                            double y,
                            double pressure,
                            double tSeconds)
{
    if (static_cast<quint64>(samples.size()) >= kSampleBlobMaxCount)
        return; // beyond any interactive stroke; drop rather than overflow
    InputSample s;
    s.x = std::isfinite(x) ? x : 0.0;
    s.y = std::isfinite(y) ? y : 0.0;
    const double p = std::isfinite(pressure) ? pressure : 1.0;
    s.pressure = static_cast<float>(std::clamp(p, 0.0, 1.0));
    double t = std::isfinite(tSeconds) ? std::max(tSeconds, 0.0) : 0.0;
    if (!samples.isEmpty()) {
        const InputSample &prev = samples.constLast();
        // NOTE THE TEST BELOW: `t <= prev.t` repairs ANY non-increasing t —
        // genuinely BACKWARD as well as exactly tied. Round-5 NIT-3: the
        // comment here said "ties", which understated the guard and left
        // callers who feed a backward t (tst_selection_gestures' post-cancel
        // appends) leaning on wording that did not cover them.
        if (t <= prev.t) {
            t = prev.t + 0.0001; // §15.1 clamped at normalization time
            // At t ≥ ~2^40 s one ulp exceeds the nudge and the add rounds
            // back to prev.t; escalate to the next representable double so
            // the strict-monotonicity duty holds at any magnitude.
            if (t <= prev.t)
                t = std::nextafter(prev.t, std::numeric_limits<double>::infinity());
        }
        const double dt = t - prev.t;
        const double dist = std::hypot(s.x - prev.x, s.y - prev.y);
        const double v = dist / dt; // dt > 0 by construction
        // Clamp into f32 range before narrowing: a huge jump over a tiny dt
        // exceeds FLT_MAX, and out-of-range narrowing is UB (and would fail
        // the blob's own finiteness validation).
        const double bounded = std::isfinite(v)
            ? std::clamp(v, 0.0, static_cast<double>(std::numeric_limits<float>::max()))
            : 0.0;
        s.velocity = static_cast<float>(bounded);
    }
    s.t = t;
    samples.append(s);
}

} // namespace appname::shell
