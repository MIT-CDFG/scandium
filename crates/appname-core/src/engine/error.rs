// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §40.1 baseline error taxonomy as proven by the proto's
//! REPORT-A suite (binding arbitration OQ-2: where v5.5 is silent, the
//! proto taxonomy wins; divergences recorded in the lane log for the
//! upstream fold-in list).
//!
//! Ported from `appname-proto` `error.rs` (same authorship), plus:
//! * `error.command.not_implemented` — binding arbitration OQ-1: emitted for
//!   registered-but-unimplemented kinds even though the v5.5 `message_ids`
//!   set lacks it (it is on the upstream mint list);
//! * `error.color.profile_space_mismatch` — the §5.5/§40.2 live-state
//!   profile check reachable from `doc.new`;
//! * `error.schema.unknown_op_kind` — the §40.6 Part-40 addition answered
//!   by `adjustment.add` for unregistered opKinds (OpNode fix-round M9,
//!   ruled: implement the contract).

use serde::Serialize;

/// Machine-readable error code. `as_str()` yields the wire string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// `schema` field present but not the `appname.command.v1` const
    /// (proto-added spelling, REPORT-A precedent).
    WrongSchema,
    /// A required field/param is absent. Spec: `error.schema.missing_field`.
    MissingField,
    /// Wrong type, out-of-range value, unknown/extra key, malformed literal.
    /// Spec: `error.schema.out_of_range` (stage-3 schema violations).
    OutOfRange,
    /// Envelope `id` already present in the journal, or nil.
    /// Spec: `error.schema.duplicate_id`.
    DuplicateId,
    /// Unregistered command kind. Spec: `error.command.unknown_kind`.
    UnknownKind,
    /// Registered kind without an implementation at this checkpoint
    /// (arbitration OQ-1). Wire: `error.command.not_implemented`.
    NotImplemented,
    /// Target does not resolve (missing layer, uninitialized /
    /// already-initialized document). Spec: `error.command.bad_target`.
    BadTarget,
    /// §11.1 lock gate at command validation (§40.5 family rules).
    /// Wire: `error.command.locked`.
    Locked,
    /// Target surface is the wrong kind for the command (§40.5, e.g. a
    /// Group where raster-editable is required).
    /// Wire: `error.command.target_kind_mismatch`.
    TargetKindMismatch,
    /// The command requires an active selection (§40.5 edit.paste_into).
    /// Wire: `error.command.selection_required`.
    SelectionRequired,
    /// §40.3: the mask slot on the target layer is already occupied.
    /// Wire: `error.command.target_in_use`.
    TargetInUse,
    /// §40.6 (new in Part 40): `adjustment.add` names an opKind absent
    /// from the §12.1/§17.1 op registry (fix-round M9 ruling: implement
    /// the contract). Wire: `error.schema.unknown_op_kind`.
    UnknownOpKind,
    /// `eraseToHistory: true` — reserved v1 param (§40.5 semantic check).
    /// Wire: `error.unsupported.erase_to_history`.
    EraseToHistoryUnsupported,
    /// §7.7 `requires.minReader` exceeds this build's reader (§51.4 clean
    /// refusal; QA-F9 closure — the registered typed id, not a generic
    /// read failure). Wire: `error.unsupported.newer_document`.
    UnsupportedNewerDocument,
    /// §45.1 sampled and pattern guidance in one `heal.stroke` payload —
    /// the contracts' mutual-exclusion cross-field (registered v5.5
    /// MessageId; CP-1.0 retouch-row O-TOOLS mechanical companion,
    /// c641056 cross-fence precedent — flagged for O-CORE ratification).
    /// Wire: `error.schema.exclusive_params`.
    ExclusiveParams,
    /// §45.1 spot healing found no valid auto-source after the widened
    /// pass — the stroke aborts atomically (§45.8 Part-46.1 row).
    /// Wire: `error.retouch.no_source_found`.
    RetouchNoSourceFound,
    /// §45.1 pattern healing referenced a deleted/missing pattern.
    /// Wire: `error.retouch.pattern_missing`.
    RetouchPatternMissing,
    /// §45.3 rule 1 — CAF fell back to boundary diffusion. NOTE: per the
    /// spec letter the COMMAND SUCCEEDS ("never a silent no-op — pixels
    /// are always produced"); this id is a WARN/toast surface, and the
    /// headless receipt-warn seam is a SEPARATE filing (O-TOOLS
    /// escalation 4, routed via O-INT) — the variant registers the wire
    /// id in the taxonomy, it is NOT a sanctioned refusal code.
    /// Wire: `error.retouch.insufficient_samples`.
    RetouchInsufficientSamples,
    /// §45.3 rule 2 — CAF had neither samples nor boundary (e.g.
    /// select-all fill on a single-layer document): the command fails
    /// atomically. Wire: `error.retouch.sample_region_empty`.
    RetouchSampleRegionEmpty,
    /// §45.3 — `sampleArea:"custom"` without a resolvable channel
    /// (`customRegion` names a missing channel; live-state resolution).
    /// Wire: `error.retouch.sample_region_missing`.
    RetouchSampleRegionMissing,
    /// §40.4 cross-document size gate (`channel.duplicate` toDocument,
    /// `channel.apply_image`/`calculations` source docId: "MUST be open
    /// with identical pixel dimensions"). Wire: `error.command.size_mismatch`.
    SizeMismatch,
    /// §14.3 spot-channel params reserved Post-1.0 (`channel.add`/
    /// `channel.edit` `ink`/`solidity` presence in v1 — classification,
    /// not a promise, §40.4). Wire: `error.unsupported.spot_channels`.
    SpotChannelsUnsupported,
    /// §5.5 profile/data-space check. Wire: `error.color.profile_space_mismatch`.
    ProfileSpaceMismatch,
    /// §5.2 profile class unusable as a working space. Wire:
    /// `error.color.profile_class`.
    ProfileClass,
    /// §5.2 ICC parse/shape failure. Wire: `error.color.profile_malformed`.
    ProfileMalformed,
    /// §5.2 ingress cap (16 MiB). Wire: `error.color.profile_too_large`.
    ProfileTooLarge,
    /// Undo requested with nothing to undo (proto-added; history.* kinds
    /// arrive at 0.7).
    NothingToUndo,
    /// Redo requested with nothing to redo (proto-added).
    NothingToRedo,
    /// Read-side IO/codec failure. Spec: `error.io.read_failed`.
    IoReadFailed,
    /// Write-side IO/codec failure. Spec: `error.io.write_failed`.
    IoWriteFailed,
    /// API-boundary misuse: unparseable JSON, non-UTF-8 bytes, envelope over
    /// the 1 MiB cap (ES §9.6). Spec: `error.api.invalid_arg`.
    InvalidArg,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::WrongSchema => "error.schema.wrong_schema",
            ErrorCode::MissingField => "error.schema.missing_field",
            ErrorCode::OutOfRange => "error.schema.out_of_range",
            ErrorCode::DuplicateId => "error.schema.duplicate_id",
            ErrorCode::UnknownKind => "error.command.unknown_kind",
            ErrorCode::NotImplemented => "error.command.not_implemented",
            ErrorCode::BadTarget => "error.command.bad_target",
            ErrorCode::Locked => "error.command.locked",
            ErrorCode::TargetKindMismatch => "error.command.target_kind_mismatch",
            ErrorCode::SelectionRequired => "error.command.selection_required",
            ErrorCode::TargetInUse => "error.command.target_in_use",
            ErrorCode::UnknownOpKind => "error.schema.unknown_op_kind",
            ErrorCode::EraseToHistoryUnsupported => "error.unsupported.erase_to_history",
            ErrorCode::UnsupportedNewerDocument => "error.unsupported.newer_document",
            ErrorCode::ExclusiveParams => "error.schema.exclusive_params",
            ErrorCode::RetouchNoSourceFound => "error.retouch.no_source_found",
            ErrorCode::RetouchPatternMissing => "error.retouch.pattern_missing",
            ErrorCode::RetouchInsufficientSamples => "error.retouch.insufficient_samples",
            ErrorCode::RetouchSampleRegionEmpty => "error.retouch.sample_region_empty",
            ErrorCode::RetouchSampleRegionMissing => "error.retouch.sample_region_missing",
            ErrorCode::SizeMismatch => "error.command.size_mismatch",
            ErrorCode::SpotChannelsUnsupported => "error.unsupported.spot_channels",
            ErrorCode::ProfileSpaceMismatch => "error.color.profile_space_mismatch",
            ErrorCode::ProfileClass => "error.color.profile_class",
            ErrorCode::ProfileMalformed => "error.color.profile_malformed",
            ErrorCode::ProfileTooLarge => "error.color.profile_too_large",
            ErrorCode::NothingToUndo => "error.history.nothing_to_undo",
            ErrorCode::NothingToRedo => "error.history.nothing_to_redo",
            ErrorCode::IoReadFailed => "error.io.read_failed",
            ErrorCode::IoWriteFailed => "error.io.write_failed",
            ErrorCode::InvalidArg => "error.api.invalid_arg",
        }
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// The one error type of the engine. Serializes to a structured JSON object
/// (`{"code": "...", "message": "...", "field": ...}`) so headless callers
/// get machine-readable rejections.
#[derive(Debug, Clone, Serialize)]
pub struct EngineError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

impl EngineError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        EngineError {
            code,
            message: message.into(),
            field: None,
        }
    }

    pub fn with_field(
        code: ErrorCode,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        EngineError {
            code,
            message: message.into(),
            field: Some(field.into()),
        }
    }

    pub fn missing(field: &str) -> Self {
        Self::with_field(
            ErrorCode::MissingField,
            field,
            format!("required field `{field}` is missing"),
        )
    }

    pub fn range(field: &str, message: impl Into<String>) -> Self {
        Self::with_field(ErrorCode::OutOfRange, field, message)
    }

    pub fn bad_target(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::BadTarget, message)
    }

    pub fn not_implemented(kind: &str, milestone: &str) -> Self {
        // O-WATCH #1 MINOR-7: no hardcoded checkpoint in the message —
        // codes are pinned wire contract, messages are prose (verified:
        // neither the adversarial corpus nor the i18n catalogs pin this
        // text; corpus expectations match `code` only).
        Self::new(
            ErrorCode::NotImplemented,
            format!("command kind `{kind}` is registered but not implemented in this build (arrives at milestone {milestone})"),
        )
    }
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.field {
            Some(field) => write!(f, "{} [{}]: {}", self.code.as_str(), field, self.message),
            None => write!(f, "{}: {}", self.code.as_str(), self.message),
        }
    }
}

impl std::error::Error for EngineError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_strings_are_the_taxonomy() {
        // The exact strings the corpus expects (fixtures/adversarial).
        assert_eq!(ErrorCode::WrongSchema.as_str(), "error.schema.wrong_schema");
        assert_eq!(
            ErrorCode::MissingField.as_str(),
            "error.schema.missing_field"
        );
        assert_eq!(ErrorCode::OutOfRange.as_str(), "error.schema.out_of_range");
        assert_eq!(ErrorCode::DuplicateId.as_str(), "error.schema.duplicate_id");
        assert_eq!(
            ErrorCode::UnknownKind.as_str(),
            "error.command.unknown_kind"
        );
        assert_eq!(
            ErrorCode::NotImplemented.as_str(),
            "error.command.not_implemented"
        );
        assert_eq!(ErrorCode::BadTarget.as_str(), "error.command.bad_target");
        assert_eq!(ErrorCode::Locked.as_str(), "error.command.locked");
        assert_eq!(
            ErrorCode::UnknownOpKind.as_str(),
            "error.schema.unknown_op_kind"
        );
        assert_eq!(
            ErrorCode::EraseToHistoryUnsupported.as_str(),
            "error.unsupported.erase_to_history"
        );
        assert_eq!(
            ErrorCode::ProfileSpaceMismatch.as_str(),
            "error.color.profile_space_mismatch"
        );
        // §45.8 retouch family (Part 46.1 taxonomy rows; registered
        // MessageIds — door slice 2 item E, the O-TOOLS escalation-4
        // ErrorCode filing).
        assert_eq!(
            ErrorCode::ExclusiveParams.as_str(),
            "error.schema.exclusive_params"
        );
        assert_eq!(
            ErrorCode::RetouchNoSourceFound.as_str(),
            "error.retouch.no_source_found"
        );
        assert_eq!(
            ErrorCode::RetouchPatternMissing.as_str(),
            "error.retouch.pattern_missing"
        );
        assert_eq!(
            ErrorCode::RetouchInsufficientSamples.as_str(),
            "error.retouch.insufficient_samples"
        );
        assert_eq!(
            ErrorCode::RetouchSampleRegionEmpty.as_str(),
            "error.retouch.sample_region_empty"
        );
        assert_eq!(
            ErrorCode::RetouchSampleRegionMissing.as_str(),
            "error.retouch.sample_region_missing"
        );
        assert_eq!(ErrorCode::InvalidArg.as_str(), "error.api.invalid_arg");
    }

    #[test]
    fn error_serializes_structured() {
        let e = EngineError::range("width", "`width` must be >= 1, got 0");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "error.schema.out_of_range");
        assert_eq!(v["field"], "width");
        let e2 = EngineError::bad_target("document already initialized");
        let v2 = serde_json::to_value(&e2).unwrap();
        assert!(v2.get("field").is_none());
    }
}
