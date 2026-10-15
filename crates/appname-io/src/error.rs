// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.1 "`CodecError` taxonomy" [NORMATIVE] — every codec failure
//! is one of these variants; the ABI status + MessageId mapping is fixed so
//! scripting and batch can branch on them. DD ⑫.
//!
//! §8.1's `MetadataInvalid` variant NEVER fails a decode: the malformed
//! block is dropped, a `CompatItem` recorded, and pixels still import.
//! Because it never crosses the API boundary, it has NO row in the §8.1
//! MessageId mapping table and therefore NO variant here (review round 1
//! blocker: no id may be invented for it — rule 4 discipline). This crate
//! represents that taxonomy arm as [`crate::registry::CompatReport`] items
//! (`shared::drop_bad_metadata`) plus the non-wire
//! [`crate::meta::MetadataInvalid`] carrier for in-crate plumbing.

use std::fmt;

/// One codec failure (§8.1 taxonomy, verbatim shape).
#[derive(Debug)]
pub enum CodecError {
    UnrecognizedFormat,
    Truncated {
        offset: u64,
    },
    Corrupt {
        detail: String,
    },
    /// e.g. "jpeg.arithmetic", "tiff.jbig", "webp.lossy_encode"
    UnsupportedFeature {
        feature: String,
    },
    LimitExceeded {
        width: u32,
        height: u32,
        max_pixels: u64,
    },
    DepthUnsupported {
        kind: appname_core::ChannelKind,
    },
    ModelUnsupported {
        model: appname_core::ColorModelKind,
    },
    // NOTE: §8.1 `MetadataInvalid` deliberately has no variant — module doc.
    OptionOutOfRange {
        field: &'static str,
        detail: String,
    },
    /// Animation export: invalid `frame:{n}` layer set (§8.2).
    FrameMapping {
        detail: String,
    },
    WriteUnsupported,
    Io(IoFailure),
    Internal(String),
}

/// Which half of the codec surface an OS-level IO failure interrupted.
/// The §8.1 [FIXED] table row for `Io` is direction-dependent:
/// `error.io.read_failed` on decode, `error.io.write_failed` on encode
/// (O-SCRIPT ABI-round review, defect 6). The call sites know their
/// direction, so it is captured at construction and the MessageId
/// accessor stays argument-free for every consumer (cabi included).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoDirection {
    /// The failure happened while reading/decoding an input stream.
    Decode,
    /// The failure happened while writing/encoding an output stream.
    Encode,
}

/// The `Io` variant payload: the OS error plus the direction it hit.
#[derive(Debug)]
pub struct IoFailure {
    pub source: std::io::Error,
    pub during: IoDirection,
}

impl CodecError {
    /// The fixed §8.1 MessageId for this variant (`Io` is direction-aware:
    /// `read_failed` on decode, `write_failed` on encode, per the
    /// [`IoDirection`] captured at construction).
    ///
    /// Codec-scoped registry ids (the Appendix-H pattern, e.g.
    /// `error.codec.png.corrupt_chunk`) key on the STABLE machine-token
    /// namespace §8.1 already defines for `feature`/detail strings
    /// ("e.g. `jpeg.arithmetic`, `tiff.jbig`"): a `psd.`-prefixed
    /// unsupported feature is a PSD-codec rejection and carries the
    /// registry row `error.codec.psd.unsupported_feature`; the PSD
    /// header parser's mode/depth-pairing reject arm emits the stable
    /// token `psd.header_invalid` and carries `error.codec.psd.
    /// header_invalid` (§42.2-A / §42.5; CR-05 keeps every OTHER header
    /// failure — signature, version, gates — on its generic §8.1 arm).
    pub fn message_id(&self) -> &'static str {
        match self {
            CodecError::UnrecognizedFormat => "error.codec.unrecognized",
            CodecError::Truncated { .. } => "error.codec.truncated",
            CodecError::Corrupt { detail } if detail.starts_with("psd.header_invalid") => {
                "error.codec.psd.header_invalid"
            }
            CodecError::Corrupt { .. } => "error.codec.corrupt",
            CodecError::UnsupportedFeature { feature } if feature.starts_with("psd.") => {
                "error.codec.psd.unsupported_feature"
            }
            // RAW-scoped registry ids (same stable-token-namespace
            // pattern as psd.*; §43.5 [NORMATIVE] table): the
            // module-absent arm is §43.5 row 1 ("RAW module not
            // installed/compiled" — a runtime-absent replaceable dylib
            // is "not installed"), the identification arm is row 2
            // (`CodecError::UnsupportedFeature` with LibRaw's error
            // string as developer detail, exactly the row's own shape).
            // Every other raw.* token (pending-slice demosaics, the
            // recorded egress gap) stays on the generic arm.
            CodecError::UnsupportedFeature { feature }
                if feature.starts_with("raw.module_unavailable") =>
            {
                "error.unsupported.raw_disabled"
            }
            CodecError::UnsupportedFeature { feature }
                if feature.starts_with("raw.unsupported_camera") =>
            {
                "error.codec.raw.unsupported_camera"
            }
            CodecError::UnsupportedFeature { .. } => "error.codec.unsupported_feature",
            CodecError::LimitExceeded { .. } => "error.codec.limit_exceeded",
            CodecError::DepthUnsupported { .. } => "error.codec.depth",
            CodecError::ModelUnsupported { .. } => "error.codec.model",
            CodecError::OptionOutOfRange { .. } => "error.codec.bad_option",
            CodecError::FrameMapping { .. } => "error.codec.frame_mapping",
            CodecError::WriteUnsupported => "error.codec.write_unsupported",
            CodecError::Io(f) => match f.during {
                IoDirection::Decode => "error.io.read_failed",
                IoDirection::Encode => "error.io.write_failed",
            },
            CodecError::Internal(_) => "error.internal",
        }
    }

    pub(crate) fn corrupt(detail: impl fmt::Display) -> CodecError {
        CodecError::Corrupt {
            detail: detail.to_string(),
        }
    }

    pub(crate) fn unsupported(feature: impl Into<String>) -> CodecError {
        CodecError::UnsupportedFeature {
            feature: feature.into(),
        }
    }

    pub(crate) fn internal(detail: impl fmt::Display) -> CodecError {
        CodecError::Internal(detail.to_string())
    }

    /// An OS IO failure on the decode (read) side → `error.io.read_failed`.
    pub(crate) fn io_read(source: std::io::Error) -> CodecError {
        CodecError::Io(IoFailure {
            source,
            during: IoDirection::Decode,
        })
    }

    /// An OS IO failure on the encode (write) side → `error.io.write_failed`.
    pub(crate) fn io_write(source: std::io::Error) -> CodecError {
        CodecError::Io(IoFailure {
            source,
            during: IoDirection::Encode,
        })
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::UnrecognizedFormat => write!(f, "unrecognized format"),
            CodecError::Truncated { offset } => write!(f, "truncated at byte {offset}"),
            CodecError::Corrupt { detail } => write!(f, "corrupt stream: {detail}"),
            CodecError::UnsupportedFeature { feature } => {
                write!(f, "unsupported feature: {feature}")
            }
            CodecError::LimitExceeded {
                width,
                height,
                max_pixels,
            } => write!(
                f,
                "{width}x{height} exceeds the decode limits (max_pixels {max_pixels})"
            ),
            CodecError::DepthUnsupported { kind } => write!(f, "unsupported depth {kind:?}"),
            CodecError::ModelUnsupported { model } => write!(f, "unsupported model {model:?}"),
            CodecError::OptionOutOfRange { field, detail } => {
                write!(f, "option `{field}` out of range: {detail}")
            }
            CodecError::FrameMapping { detail } => write!(f, "frame mapping: {detail}"),
            CodecError::WriteUnsupported => write!(f, "write unsupported"),
            CodecError::Io(io) => write!(f, "io: {}", io.source),
            CodecError::Internal(detail) => write!(f, "internal: {detail}"),
        }
    }
}

impl std::error::Error for CodecError {}

// NOTE: deliberately NO blanket `From<std::io::Error>` — a bare `?` cannot
// know which §8.1 direction it is on. Every construction goes through
// [`CodecError::io_read`] / [`CodecError::io_write`] (or builds the
// [`IoFailure`] payload explicitly), so the direction is always stated
// where it is known: the call site.

#[cfg(test)]
mod tests {
    use super::*;

    fn os_err() -> std::io::Error {
        std::io::Error::other("disk full")
    }

    /// O-SCRIPT ABI-round review, defect 6: the §8.1 [FIXED] `Io` row is
    /// direction-dependent. An encoder-side OS failure MUST surface as
    /// `error.io.write_failed` through the codec error surface — the
    /// crate-level leg of the mapping (the ABI-side leg lives in cabi's
    /// conformance suite). Both ids exist in `contracts/message_ids.yaml`;
    /// nothing minted here.
    #[test]
    fn encoder_io_error_surfaces_write_failed() {
        let err = CodecError::io_write(os_err());
        assert_eq!(err.message_id(), "error.io.write_failed");
        // Display carries the OS detail, direction rides only in the id.
        assert_eq!(err.to_string(), "io: disk full");
    }

    /// The decode arm keeps its pre-fix mapping byte-stable.
    #[test]
    fn decoder_io_error_surfaces_read_failed() {
        let err = CodecError::io_read(os_err());
        assert_eq!(err.message_id(), "error.io.read_failed");
        assert_eq!(err.to_string(), "io: disk full");
    }
}
