// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §15.1 (normalized input samples), §9.2 (ingress — landed in
// the ABI as `appname_asset_create` taking the canonical blob BYTES),
// §9.6 rule 2 (samples NEVER inline in envelope JSON — the `samples`
// field is an assetId). Byte format
// authority: crates/appname-tools/src/brush/sample.rs module docs
// ("appname.samples.v1", version 1, little-endian, 16-byte header +
// 48-byte records, reserved bytes canonical zero). Plan §2.2 "Qt/QML shell
// v1" row (pointer events -> samples.v1 blobs).
//
// This is the SHELL-side encoder: it builds the canonical blob bytes and
// the §9.2 C-mirror record array from pointer input. It performs no
// engine math — validation ranges mirror the documented contract so the
// shell never hands the engine bytes it would reject.

#pragma once

#include <QByteArray>
#include <QString>
#include <QVector>

#include <cstddef>
#include <cstdint>
#include <optional>

namespace appname::shell {

// Field-for-field the §9.2 `AppnameInputSampleC` layout (which is also,
// with its natural-alignment hole at +36 canonicalized to zero, the
// 48-byte record of the `appname.samples.v1` blob). The static_asserts
// below pin the layout. The additive ABI block landed as a blob-bytes API
// (`appname_asset_create`) WITHOUT shipping this struct in the header, so
// this local mirror stays: it is the shell's record-layout authority for
// encoding the blob (byte-format authority: sample.rs, see below).
struct InputSample {
    double x = 0.0;        // +0  document-space px, sub-pixel
    double y = 0.0;        // +8  document-space px, sub-pixel
    float pressure = 1.0f; // +16 0..=1 (post-calibration; mouse = 1.0)
    float tiltX = 0.0f;    // +20 -1..=1
    float tiltY = 0.0f;    // +24 -1..=1
    float rotation = 0.0f; // +28 barrel rotation, radians
    float velocity = 0.0f; // +32 px/s, derived, >= 0
    // +36..40: natural-alignment hole — blob writes it as 0x00000000
    double t = 0.0;        // +40 seconds since stroke start, strictly increasing
};

static_assert(sizeof(InputSample) == 48, "record layout must be 48 bytes");
static_assert(offsetof(InputSample, x) == 0);
static_assert(offsetof(InputSample, y) == 8);
static_assert(offsetof(InputSample, pressure) == 16);
static_assert(offsetof(InputSample, tiltX) == 20);
static_assert(offsetof(InputSample, tiltY) == 24);
static_assert(offsetof(InputSample, rotation) == 28);
static_assert(offsetof(InputSample, velocity) == 32);
static_assert(offsetof(InputSample, t) == 40);

inline constexpr char kSampleBlobMagic[8] = {'A', 'P', 'N', 'S', 'A', 'M', 'P', 'L'};
inline constexpr quint16 kSampleBlobVersion = 1;
inline constexpr int kSampleBlobHeaderBytes = 16;
inline constexpr int kSampleRecordBytes = 48;
inline constexpr quint32 kSampleBlobMaxCount = 1'048'576;

// Validation verdict; `ok()` empty message == valid. Messages are
// developer-facing diagnostics (status-bar surfacing goes through the
// i18n catalog with the code as an argument).
struct SampleValidation {
    bool valid = false;
    QString detail; // developer detail, untranslated by design
};

// Mirror of the documented per-field contract (§15.1 ranges, strict
// monotonic t, finiteness, count 1..=1_048_576).
SampleValidation validateSamples(const QVector<InputSample> &samples);

// Canonical `appname.samples.v1` bytes (validates first; std::nullopt on
// invalid input). Bit-exact vs the documented layout — unit-tested against
// hand-built golden bytes.
std::optional<QByteArray> encodeSamplesBlob(const QVector<InputSample> &samples);

// Input normalization helper: append a pointer-event sample enforcing the
// §15.1 upstream duties — clamped pressure/tilt, strictly increasing t
// (round-5 NIT-3: ANY non-increasing t is nudged to prev.t + 0.0001 s —
// genuinely backward as well as exactly tied; the guard is `t <= prev.t`,
// escalating to the next representable double when the nudge would round
// away. This is the "clamped at normalization time" rule, and it is
// strictly wider than the "ties" the old wording claimed. validateSamples
// REJECTS a backward t outright, so the difference is load-bearing for any
// caller that relies on a repair rather than a refusal — pinned by
// tst_sample_blob_adversarial's tRegressionRejectedRawNudgedNormalized),
// derived velocity (px/s vs the previous sample; 0 for the first).
void appendNormalizedSample(QVector<InputSample> &samples,
                            double x,
                            double y,
                            double pressure,
                            double tSeconds);

} // namespace appname::shell
