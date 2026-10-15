// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §15.1 (normalized tablet input `AppnameInputSample`), §9.2
//! (`AppnameInputSampleC` C mirror + `appname_samples_create`), §9.6 rule 2
//! (samples NEVER travel inline in JSON — the envelope's `samples` field is
//! an assetId); contracts `brush.stroke.yaml` / `eraser.stroke.yaml`
//! stroke-family rules. DD ⑮.
//!
//! # The `appname.samples.v1` binary blob
//!
//! The byte format of an `AppnameInputSample` asset — the blob that
//! `appname_samples_create` (§9.2) copies into the document asset store and
//! that a stroke envelope's `samples` field references by assetId (§9.6
//! rule 2; proto divergence "stroke samples ride inline in `params`" closes
//! at CP-0.3 with this format). The package-asset storage side (ingestion,
//! GC with history, `.appname` carriage) is O-CORE's; this module owns the
//! *bytes contract* and its parser/serializer/validator.
//!
//! ## Layout (version 1, all integers and floats little-endian)
//!
//! ```text
//! header — 16 bytes
//!   0..8    magic            = b"APNSAMPL" (ASCII)
//!   8..10   version: u16 LE  = 1
//!   10..12  flags:   u16 LE  = 0            (v1 defines no flags; nonzero rejects)
//!   12..16  count:   u32 LE  ∈ 1..=1_048_576
//! records — count × 48 bytes, field-for-field the §9.2 `AppnameInputSampleC`
//! layout including its natural-alignment padding, canonicalized to zero:
//!   +0..8    x:        f64 LE  document-space px, sub-pixel (finite)
//!   +8..16   y:        f64 LE  document-space px, sub-pixel (finite)
//!   +16..20  pressure: f32 LE  0..=1 (post-calibration, §15.1)
//!   +20..24  tilt_x:   f32 LE  -1..=1
//!   +24..28  tilt_y:   f32 LE  -1..=1
//!   +28..32  rotation: f32 LE  barrel rotation, radians (finite, unbounded;
//!                              normalized mod 2π at dynamics eval, §15.4)
//!   +32..36  velocity: f32 LE  px/s, derived (§15.1), finite, >= 0
//!   +36..40  reserved         = 0x00000000 (the C struct's alignment hole,
//!                              canonicalized so blob bytes are content-addressable)
//!   +40..48  t:        f64 LE  seconds since stroke start, finite, >= 0,
//!                              strictly increasing (§15.1 monotonic-`t` rule;
//!                              ties are clamped at normalization time upstream)
//! total = 16 + 48·count bytes, exactly (trailing bytes reject)
//! ```
//!
//! Strictness is deliberate: assets are content-addressed in the package, so
//! one sample stream must have exactly one encoding (reserved bytes zero,
//! flags zero, no trailing bytes). A blob failing any check here is "not an
//! `AppnameInputSample` asset" — O-CORE maps that to stage-4
//! `error.command.bad_target` per the §40.5 family rule.

use std::fmt;

/// One normalized input sample (ES §15.1; field-for-field the Rust struct
/// the spec defines, and the §9.2 C mirror).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputSample {
    /// Document space, sub-pixel.
    pub x: f64,
    /// Document space, sub-pixel.
    pub y: f64,
    /// `0..=1`, post-calibration (§15.1).
    pub pressure: f32,
    /// `-1..=1`.
    pub tilt_x: f32,
    /// `-1..=1`.
    pub tilt_y: f32,
    /// Barrel rotation, radians.
    pub rotation: f32,
    /// px/s, derived (§15.1).
    pub velocity: f32,
    /// Seconds since stroke start; strictly increasing per stroke (§15.1).
    pub t: f64,
}

/// Blob magic, first 8 bytes.
pub const SAMPLE_BLOB_MAGIC: [u8; 8] = *b"APNSAMPL";
/// Format version this module reads and writes.
pub const SAMPLE_BLOB_VERSION: u16 = 1;
/// Header size in bytes.
pub const SAMPLE_BLOB_HEADER_BYTES: usize = 16;
/// Per-record size in bytes (§9.2 `AppnameInputSampleC` with its natural
/// padding, canonical zero).
pub const SAMPLE_RECORD_BYTES: usize = 48;
/// v1 allocation guard: 2²⁰ samples ≈ 73 min of 240 Hz input ≈ 48 MiB.
pub const SAMPLE_BLOB_MAX_COUNT: u32 = 1_048_576;

/// Which sample field failed validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleField {
    X,
    Y,
    Pressure,
    TiltX,
    TiltY,
    Rotation,
    Velocity,
    T,
}

/// Everything that can be wrong with a sample blob or sample slice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SampleBlobError {
    /// Shorter than the 16-byte header.
    TooShort { len: usize },
    /// First 8 bytes are not [`SAMPLE_BLOB_MAGIC`].
    BadMagic,
    /// Header version is not [`SAMPLE_BLOB_VERSION`].
    UnsupportedVersion(u16),
    /// v1 defines no flags; any nonzero bit rejects.
    UnsupportedFlags(u16),
    /// Byte length is not exactly `16 + 48·count`.
    LengthMismatch { expected: usize, actual: usize },
    /// `count == 0` — a stroke has at least its pen-down sample (§15.3).
    CountZero,
    /// `count` exceeds [`SAMPLE_BLOB_MAX_COUNT`].
    CountOverMax { count: u32 },
    /// A record's reserved bytes (offset 36..40) are not zero.
    ReservedNonZero { index: u32 },
    /// A float field is NaN or infinite.
    NonFinite { index: u32, field: SampleField },
    /// A field is outside its documented range.
    OutOfRange { index: u32, field: SampleField },
    /// `t` failed the strictly-increasing rule (§15.1) at this record.
    TimeNotIncreasing { index: u32 },
}

impl fmt::Display for SampleBlobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SampleBlobError::TooShort { len } => {
                write!(f, "sample blob too short for header: {len} bytes")
            }
            SampleBlobError::BadMagic => write!(f, "sample blob magic mismatch"),
            SampleBlobError::UnsupportedVersion(v) => {
                write!(f, "sample blob version {v} unsupported (expect 1)")
            }
            SampleBlobError::UnsupportedFlags(v) => {
                write!(
                    f,
                    "sample blob flags {v:#06x} unsupported (v1 defines none)"
                )
            }
            SampleBlobError::LengthMismatch { expected, actual } => {
                write!(f, "sample blob length {actual} != expected {expected}")
            }
            SampleBlobError::CountZero => write!(f, "sample blob has zero samples"),
            SampleBlobError::CountOverMax { count } => {
                write!(
                    f,
                    "sample count {count} over v1 maximum {SAMPLE_BLOB_MAX_COUNT}"
                )
            }
            SampleBlobError::ReservedNonZero { index } => {
                write!(f, "record {index}: reserved bytes not zero")
            }
            SampleBlobError::NonFinite { index, field } => {
                write!(f, "record {index}: field {field:?} not finite")
            }
            SampleBlobError::OutOfRange { index, field } => {
                write!(f, "record {index}: field {field:?} out of range")
            }
            SampleBlobError::TimeNotIncreasing { index } => {
                write!(f, "record {index}: t not strictly increasing")
            }
        }
    }
}

impl std::error::Error for SampleBlobError {}

/// Validate a sample slice against the §15.1 field contracts (the same
/// checks `decode_samples` applies; `encode_samples` refuses to produce a
/// blob that would not decode).
pub fn validate_samples(samples: &[InputSample]) -> Result<(), SampleBlobError> {
    if samples.is_empty() {
        return Err(SampleBlobError::CountZero);
    }
    if samples.len() as u64 > SAMPLE_BLOB_MAX_COUNT as u64 {
        return Err(SampleBlobError::CountOverMax {
            count: samples.len().min(u32::MAX as usize) as u32,
        });
    }
    let mut prev_t = f64::NEG_INFINITY;
    for (i, s) in samples.iter().enumerate() {
        let index = i as u32;
        let finite = [
            (s.x.is_finite(), SampleField::X),
            (s.y.is_finite(), SampleField::Y),
            (s.pressure.is_finite(), SampleField::Pressure),
            (s.tilt_x.is_finite(), SampleField::TiltX),
            (s.tilt_y.is_finite(), SampleField::TiltY),
            (s.rotation.is_finite(), SampleField::Rotation),
            (s.velocity.is_finite(), SampleField::Velocity),
            (s.t.is_finite(), SampleField::T),
        ];
        for (ok, field) in finite {
            if !ok {
                return Err(SampleBlobError::NonFinite { index, field });
            }
        }
        if !(0.0..=1.0).contains(&s.pressure) {
            return Err(SampleBlobError::OutOfRange {
                index,
                field: SampleField::Pressure,
            });
        }
        if !(-1.0..=1.0).contains(&s.tilt_x) {
            return Err(SampleBlobError::OutOfRange {
                index,
                field: SampleField::TiltX,
            });
        }
        if !(-1.0..=1.0).contains(&s.tilt_y) {
            return Err(SampleBlobError::OutOfRange {
                index,
                field: SampleField::TiltY,
            });
        }
        if s.velocity < 0.0 {
            return Err(SampleBlobError::OutOfRange {
                index,
                field: SampleField::Velocity,
            });
        }
        if s.t < 0.0 {
            return Err(SampleBlobError::OutOfRange {
                index,
                field: SampleField::T,
            });
        }
        if s.t <= prev_t {
            return Err(SampleBlobError::TimeNotIncreasing { index });
        }
        prev_t = s.t;
    }
    Ok(())
}

/// Serialize samples into canonical `appname.samples.v1` bytes.
/// Validates first — never emits a blob `decode_samples` would reject.
pub fn encode_samples(samples: &[InputSample]) -> Result<Vec<u8>, SampleBlobError> {
    validate_samples(samples)?;
    let mut out =
        Vec::with_capacity(SAMPLE_BLOB_HEADER_BYTES + SAMPLE_RECORD_BYTES * samples.len());
    out.extend_from_slice(&SAMPLE_BLOB_MAGIC);
    out.extend_from_slice(&SAMPLE_BLOB_VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // flags
    out.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.x.to_le_bytes());
        out.extend_from_slice(&s.y.to_le_bytes());
        out.extend_from_slice(&s.pressure.to_le_bytes());
        out.extend_from_slice(&s.tilt_x.to_le_bytes());
        out.extend_from_slice(&s.tilt_y.to_le_bytes());
        out.extend_from_slice(&s.rotation.to_le_bytes());
        out.extend_from_slice(&s.velocity.to_le_bytes());
        out.extend_from_slice(&[0u8; 4]); // reserved (canonical zero)
        out.extend_from_slice(&s.t.to_le_bytes());
    }
    Ok(out)
}

/// Parse + validate `appname.samples.v1` bytes.
pub fn decode_samples(bytes: &[u8]) -> Result<Vec<InputSample>, SampleBlobError> {
    if bytes.len() < SAMPLE_BLOB_HEADER_BYTES {
        return Err(SampleBlobError::TooShort { len: bytes.len() });
    }
    if bytes[0..8] != SAMPLE_BLOB_MAGIC {
        return Err(SampleBlobError::BadMagic);
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != SAMPLE_BLOB_VERSION {
        return Err(SampleBlobError::UnsupportedVersion(version));
    }
    let flags = u16::from_le_bytes([bytes[10], bytes[11]]);
    if flags != 0 {
        return Err(SampleBlobError::UnsupportedFlags(flags));
    }
    let count = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    if count == 0 {
        return Err(SampleBlobError::CountZero);
    }
    if count > SAMPLE_BLOB_MAX_COUNT {
        return Err(SampleBlobError::CountOverMax { count });
    }
    let expected = SAMPLE_BLOB_HEADER_BYTES + SAMPLE_RECORD_BYTES * count as usize;
    if bytes.len() != expected {
        return Err(SampleBlobError::LengthMismatch {
            expected,
            actual: bytes.len(),
        });
    }
    let mut samples = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let r = &bytes[SAMPLE_BLOB_HEADER_BYTES + i * SAMPLE_RECORD_BYTES..];
        if r[36..40] != [0u8; 4] {
            return Err(SampleBlobError::ReservedNonZero { index: i as u32 });
        }
        let f64_at = |o: usize| f64::from_le_bytes(r[o..o + 8].try_into().expect("8 bytes"));
        let f32_at = |o: usize| f32::from_le_bytes(r[o..o + 4].try_into().expect("4 bytes"));
        samples.push(InputSample {
            x: f64_at(0),
            y: f64_at(8),
            pressure: f32_at(16),
            tilt_x: f32_at(20),
            tilt_y: f32_at(24),
            rotation: f32_at(28),
            velocity: f32_at(32),
            t: f64_at(40),
        });
    }
    validate_samples(&samples)?;
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(x: f64, y: f64, pressure: f32, t: f64) -> InputSample {
        InputSample {
            x,
            y,
            pressure,
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.0,
            velocity: 0.0,
            t,
        }
    }

    fn good() -> Vec<InputSample> {
        vec![
            sample(0.0, 0.0, 0.5, 0.0),
            sample(10.0, 5.0, 0.75, 0.008),
            sample(20.0, 10.0, 1.0, 0.016),
        ]
    }

    #[test]
    fn round_trip_is_bit_identical() {
        let blob = encode_samples(&good()).unwrap();
        assert_eq!(
            blob.len(),
            SAMPLE_BLOB_HEADER_BYTES + 3 * SAMPLE_RECORD_BYTES
        );
        let back = decode_samples(&blob).unwrap();
        assert_eq!(back, good());
        // Re-encode reproduces the exact bytes (canonical encoding).
        assert_eq!(encode_samples(&back).unwrap(), blob);
    }

    #[test]
    fn header_fields_are_as_documented() {
        let blob = encode_samples(&good()).unwrap();
        assert_eq!(&blob[0..8], b"APNSAMPL");
        assert_eq!(u16::from_le_bytes([blob[8], blob[9]]), 1);
        assert_eq!(u16::from_le_bytes([blob[10], blob[11]]), 0);
        assert_eq!(
            u32::from_le_bytes([blob[12], blob[13], blob[14], blob[15]]),
            3
        );
    }

    #[test]
    fn truncation_and_trailing_bytes_reject() {
        let blob = encode_samples(&good()).unwrap();
        assert_eq!(
            decode_samples(&blob[..10]),
            Err(SampleBlobError::TooShort { len: 10 })
        );
        let torn = &blob[..blob.len() - 1];
        assert_eq!(
            decode_samples(torn),
            Err(SampleBlobError::LengthMismatch {
                expected: blob.len(),
                actual: blob.len() - 1
            })
        );
        let mut padded = blob.clone();
        padded.push(0);
        assert_eq!(
            decode_samples(&padded),
            Err(SampleBlobError::LengthMismatch {
                expected: blob.len(),
                actual: blob.len() + 1
            })
        );
    }

    #[test]
    fn bad_magic_version_flags_reject() {
        let blob = encode_samples(&good()).unwrap();
        let mut b = blob.clone();
        b[0] = b'X';
        assert_eq!(decode_samples(&b), Err(SampleBlobError::BadMagic));
        let mut b = blob.clone();
        b[8] = 2;
        assert_eq!(
            decode_samples(&b),
            Err(SampleBlobError::UnsupportedVersion(2))
        );
        let mut b = blob.clone();
        b[10] = 1;
        assert_eq!(
            decode_samples(&b),
            Err(SampleBlobError::UnsupportedFlags(1))
        );
    }

    #[test]
    fn zero_count_rejects() {
        let mut b = Vec::new();
        b.extend_from_slice(&SAMPLE_BLOB_MAGIC);
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(decode_samples(&b), Err(SampleBlobError::CountZero));
        assert_eq!(encode_samples(&[]), Err(SampleBlobError::CountZero));
    }

    #[test]
    fn count_over_max_rejects_without_allocating() {
        let mut b = Vec::new();
        b.extend_from_slice(&SAMPLE_BLOB_MAGIC);
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&(SAMPLE_BLOB_MAX_COUNT + 1).to_le_bytes());
        assert_eq!(
            decode_samples(&b),
            Err(SampleBlobError::CountOverMax {
                count: SAMPLE_BLOB_MAX_COUNT + 1
            })
        );
    }

    #[test]
    fn reserved_bytes_must_be_zero() {
        let mut blob = encode_samples(&good()).unwrap();
        blob[SAMPLE_BLOB_HEADER_BYTES + 37] = 0xAB;
        assert_eq!(
            decode_samples(&blob),
            Err(SampleBlobError::ReservedNonZero { index: 0 })
        );
    }

    #[test]
    fn field_ranges_reject() {
        let mut s = good();
        s[1].pressure = 1.5;
        assert_eq!(
            validate_samples(&s),
            Err(SampleBlobError::OutOfRange {
                index: 1,
                field: SampleField::Pressure
            })
        );
        let mut s = good();
        s[0].tilt_x = -1.01;
        assert_eq!(
            validate_samples(&s),
            Err(SampleBlobError::OutOfRange {
                index: 0,
                field: SampleField::TiltX
            })
        );
        let mut s = good();
        s[2].velocity = -0.1;
        assert_eq!(
            validate_samples(&s),
            Err(SampleBlobError::OutOfRange {
                index: 2,
                field: SampleField::Velocity
            })
        );
        let mut s = good();
        s[0].t = -0.5;
        assert_eq!(
            validate_samples(&s),
            Err(SampleBlobError::OutOfRange {
                index: 0,
                field: SampleField::T
            })
        );
    }

    #[test]
    fn non_finite_rejects() {
        let mut s = good();
        s[1].x = f64::NAN;
        assert_eq!(
            validate_samples(&s),
            Err(SampleBlobError::NonFinite {
                index: 1,
                field: SampleField::X
            })
        );
        let mut s = good();
        s[2].rotation = f32::INFINITY;
        assert_eq!(
            validate_samples(&s),
            Err(SampleBlobError::NonFinite {
                index: 2,
                field: SampleField::Rotation
            })
        );
    }

    #[test]
    fn time_must_strictly_increase() {
        let mut s = good();
        s[2].t = s[1].t; // tie
        assert_eq!(
            validate_samples(&s),
            Err(SampleBlobError::TimeNotIncreasing { index: 2 })
        );
        let mut s = good();
        s[1].t = 1.0;
        s[2].t = 0.5; // regression
        assert_eq!(
            validate_samples(&s),
            Err(SampleBlobError::TimeNotIncreasing { index: 2 })
        );
    }

    #[test]
    fn single_sample_blob_is_valid() {
        let one = vec![sample(3.5, -2.25, 1.0, 0.0)];
        let blob = encode_samples(&one).unwrap();
        assert_eq!(decode_samples(&blob).unwrap(), one);
    }
}
