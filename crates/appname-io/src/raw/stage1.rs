// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Stage 1 — the sanitized LibRaw wrapper boundary (§43.4 rule 1: "Stage
//! 1 (open/unpack) is LibRaw behind the fuzzed, sanitized wrapper";
//! §34.2 `raw_probe` names this exact boundary).
//!
//! Spec: ES §8.3 stage 1 [NORMATIVE]; Part 43 §43.4 stage ownership,
//! §43.5 unsupported-camera/absent-module table; ES §1.4 (the facade is
//! `sys/libraw-sys`, this module is its ONLY consumer). Record:
//! `deps/libraw.yaml` (merged; this slice is its `preAuthorizedFor`
//! consumption).
//!
//! **Module resolution (the replaceable-dylib posture).** The LibRaw
//! module is dynamically loaded at RUNTIME from an explicit path — never
//! a default-name system search (the record treats Part 43 rule 1's
//! pinned-version determinism as controlling; an arbitrary system libraw
//! silently breaks the doubly-pinned goldens). Resolution order:
//! 1. a module installed through [`install_module`] (the door the
//!    packaging channel / shell wires once the dmg wave decides the
//!    shipped dylib's source — the record's `packagingChannel` gap);
//! 2. else, one lazy attempt at `APPNAME_LIBRAW_DYLIB` (development /
//!    LGPL-2.1 §6 user-substitution seam);
//! 3. else ABSENT: decode refuses typed with the §43.5 row-1 arm
//!    (`error.unsupported.raw_disabled` — "not installed").
//!
//! **Error mapping without numeric codes.** LibRaw's numeric error
//! values are UNDOCUMENTED (facade doc), so mapping branches on CALL
//! SITE, never on code values: `open_buffer` failure = LibRaw could not
//! identify the format → `error.codec.raw.unsupported_camera` with the
//! `strerror` text as developer detail (§43.5 row 2, never parsed);
//! `unpack` failure = identified but bad data → `error.codec.corrupt`
//! (§43.5 row 4). Never a crash (§43.5).
//!
//! **The mosaic-egress gap (RECORD-LEVEL FINDING, escalated).** The
//! documented LibRaw C API (read first-hand for this slice — facade doc)
//! exposes NO accessor for the raw CFA pixel plane, per-channel black
//! levels, the orientation flag, or the EXIF block; `libraw_data_t` is
//! not layout-documented, and the record's Part 47 posture ("no Appname
//! implementer reads LibRaw source") forecloses declaring its internals
//! from the header. Everything BEFORE that hop is live below (open,
//! unpack, gates, CFA classification, WB multipliers, white level,
//! matrix derivation — each sanitized); the mosaic hop itself refuses
//! typed (`raw.stage1_mosaic_egress_undocumented_c_api`) pending an
//! O-INT/O-BUILD ruling on the options recorded in the lane log (own C
//! shim at the packaging channel, an interface-header carve-out ruling,
//! or an upstreamed accessor). Nothing is improvised here.

use std::sync::OnceLock;

use libraw_sys::{RawLib, RawShot};

use crate::error::CodecError;
use crate::opts::DecodeOpts;
use crate::raw::develop::{CfaPattern, MatrixSource};

/// Environment variable naming the LibRaw dylib for development runs and
/// LGPL §6 substitution (resolution order in the module doc).
pub const MODULE_ENV: &str = "APPNAME_LIBRAW_DYLIB";

enum ModuleState {
    Loaded(RawLib),
    Absent(String),
}

static MODULE: OnceLock<ModuleState> = OnceLock::new();

/// Install the process-wide LibRaw module (the packaging-channel door;
/// module doc). First install wins; a second call returns the rejected
/// module back to the caller (boxed — the table is large).
pub fn install_module(lib: RawLib) -> Result<(), Box<RawLib>> {
    let mut holder = Some(lib);
    let _ = MODULE.get_or_init(|| ModuleState::Loaded(holder.take().expect("first init")));
    match holder {
        None => Ok(()),
        Some(rejected) => Err(Box::new(rejected)),
    }
}

fn resolve_global() -> &'static ModuleState {
    MODULE.get_or_init(|| match std::env::var(MODULE_ENV) {
        Ok(path) if !path.is_empty() => {
            match RawLib::load(std::path::Path::new(&path)) {
                Ok(lib) => ModuleState::Loaded(lib),
                // A named-but-unloadable module is still ABSENT (§48.4
                // clean typed error) — the loader detail rides along.
                Err(e) => ModuleState::Absent(format!("{MODULE_ENV}: {e}")),
            }
        }
        _ => ModuleState::Absent(format!(
            "no module installed and {MODULE_ENV} is unset (packaging channel undecided — deps/libraw.yaml packagingChannel)"
        )),
    })
}

/// The globally resolved module, if present (open-dialog filtering and
/// the codec share this answer).
pub fn global_module() -> Result<&'static RawLib, CodecError> {
    match resolve_global() {
        ModuleState::Loaded(lib) => Ok(lib),
        ModuleState::Absent(detail) => Err(module_unavailable(detail)),
    }
}

/// The §43.5 row-1 typed refusal ("RAW module not installed") —
/// maps to `error.unsupported.raw_disabled` via the token namespace.
pub(crate) fn module_unavailable(detail: &str) -> CodecError {
    CodecError::unsupported(format!("raw.module_unavailable: {detail}"))
}

/// Everything stage 1 can hand over at this slice (sanitized): the
/// §8.3 stage-1 output contract MINUS the mosaic plane / black levels /
/// orientation / EXIF, which sit behind the recorded egress gap
/// (module doc).
#[derive(Debug)]
pub struct Stage1Meta {
    /// Active-area size.
    pub width: u32,
    pub height: u32,
    /// Bayer pattern (non-Bayer refuses earlier, typed).
    pub pattern: CfaPattern,
    /// As-shot WB multipliers `(r, g1, b, g2)`; sanitized (finite,
    /// positive r/g1/b) or neutralized with a report note.
    pub cam_mul: [f32; 4],
    /// `true` when `cam_mul` had to be neutralized (caller records the
    /// `Approximated` report item).
    pub cam_mul_neutralized: bool,
    /// Saturation/white level (> 0, finite — the degenerate-levels
    /// advisory's first gate).
    pub white_level: f32,
    /// Camera-RGB → XYZ(D65) derived from LibRaw's `rgb_cam` when
    /// finite and invertible; `None` = the §43.2 rule-3 sRGB assumption
    /// (caller records the RAW-02 item).
    pub cam_to_xyz_d65: Option<[[f32; 3]; 3]>,
    /// §43.2 `matrixSource` for the recorded block. NOTE (recorded
    /// caveat): the documented C API cannot distinguish rule 1
    /// (embedded matrix) from rule 2 (LibRaw table) — both surface as
    /// `rgb_cam` — so a usable matrix is recorded as `LibRawTable`;
    /// flagged in the lane log for the fold-in register.
    pub matrix_source: MatrixSource,
    /// The LOADED module's version string (Part 43 determinism claims
    /// key on the pinned `libraw_sys::PINNED_VERSION`).
    pub libraw_version: String,
}

/// Open + unpack + sanitize (stages 1's live half). Bomb gates fire
/// from header-derived dimensions BEFORE any further work (§8.1).
pub fn stage1_open(
    lib: &RawLib,
    bytes: &[u8],
    opts: &DecodeOpts,
) -> Result<Stage1Meta, CodecError> {
    // Open: identification failure → §43.5 row 2.
    let mut shot = lib
        .open_buffer(bytes)
        .map_err(|e| CodecError::unsupported(format!("raw.unsupported_camera: {}", e.message)))?;
    // Unpack: identified but undecodable → corrupt (§43.5 row 4;
    // call-site mapping, module doc).
    shot.unpack()
        .map_err(|e| CodecError::corrupt(format!("raw.unpack: {}", e.message)))?;

    // §8.1 decompression-bomb gates from the decoder's own header-derived
    // dimensions, before any pixel-sized work.
    let (w, h) = (shot.width(), shot.height());
    if w <= 0 || h <= 0 {
        return Err(CodecError::corrupt(format!(
            "raw.dimensions: non-positive active area {w}x{h}"
        )));
    }
    let (w, h) = (w as u32, h as u32);
    opts.check_limits(w, h)?;

    // CFA classification through the documented probe (libraw_COLOR):
    // 2×2-periodic over a 4×4 window with codes in {R,G,B,G2} = Bayer;
    // anything else is the non-Bayer delegation class (typed for now —
    // the §8.3 delegation path is recorded as arriving with the egress
    // resolution, lane log).
    let pattern = classify_cfa(&shot)?;

    // As-shot multipliers, sanitized (metadata garbage never fails an
    // import — neutralized + reported instead).
    let raw_mul = shot.cam_mul();
    let usable = raw_mul[0].is_finite()
        && raw_mul[1].is_finite()
        && raw_mul[2].is_finite()
        && raw_mul[3].is_finite()
        && raw_mul[0] > 0.0
        && raw_mul[1] > 0.0
        && raw_mul[2] > 0.0
        && raw_mul[3] >= 0.0;
    let (cam_mul, cam_mul_neutralized) = if usable {
        (raw_mul, false)
    } else {
        ([1.0, 1.0, 1.0, 0.0], true)
    };

    // White level — the degenerate-levels advisory's stage-1 gate
    // (finite, > 0); per-position black levels join at egress
    // resolution and pass develop::sanitize_levels there.
    let max = shot.color_maximum();
    if max <= 0 {
        return Err(CodecError::corrupt(format!(
            "raw.levels_degenerate: white level {max} (advisory: sanitize before the codec boundary)"
        )));
    }

    // Camera matrix: rgb_cam (camera→sRGB D65) when finite and
    // non-singular → cam_to_xyz(D65) = SRGB_TO_XYZ · rgb_cam.
    let rgb_cam = shot.rgb_cam();
    let cam_to_xyz_d65 = derive_cam_to_xyz(&rgb_cam);
    let matrix_source = if cam_to_xyz_d65.is_some() {
        MatrixSource::LibRawTable
    } else {
        MatrixSource::AssumedSrgb
    };

    Ok(Stage1Meta {
        width: w,
        height: h,
        pattern,
        cam_mul,
        cam_mul_neutralized,
        white_level: max as f32,
        cam_to_xyz_d65,
        matrix_source,
        libraw_version: lib.version(),
    })
}

/// The recorded egress gap (module doc): typed, never silent, never a
/// stand-in decode.
pub fn mosaic_egress_gap() -> CodecError {
    CodecError::unsupported("raw.stage1_mosaic_egress_undocumented_c_api".to_owned())
}

fn classify_cfa(shot: &RawShot<'_>) -> Result<CfaPattern, CodecError> {
    // Probe a 4×4 window; validate codes and 2×2 periodicity.
    let mut probe = [[0i32; 4]; 4];
    for (r, row) in probe.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = shot.cfa_color(r as i32, c as i32);
        }
    }
    let mut periodic = true;
    for r in 0..4 {
        for c in 0..4 {
            if probe[r][c] != probe[r % 2][c % 2] || !(0..=3).contains(&probe[r][c]) {
                periodic = false;
            }
        }
    }
    if periodic {
        // Fold G2 (3) → G (1) and map the tile to a CfaPattern.
        let fold = |v: i32| if v == 3 { 1 } else { v as usize };
        let tile = [
            fold(probe[0][0]),
            fold(probe[0][1]),
            fold(probe[1][0]),
            fold(probe[1][1]),
        ];
        let pattern = match tile {
            [0, 1, 1, 2] => Some(CfaPattern::Rggb),
            [2, 1, 1, 0] => Some(CfaPattern::Bggr),
            [1, 0, 2, 1] => Some(CfaPattern::Grbg),
            [1, 2, 0, 1] => Some(CfaPattern::Gbrg),
            _ => None,
        };
        if let Some(p) = pattern {
            return Ok(p);
        }
    }
    // Non-Bayer (X-Trans/Foveon) or unrecognizable: the §8.3 delegation
    // class — typed until the egress resolution lands the delegation
    // path (lane-log deferral; the substitution report item arrives
    // with it).
    Err(CodecError::unsupported(
        "raw.non_bayer_delegation_pending_egress_resolution".to_owned(),
    ))
}

/// `cam_to_xyz(D65) = SRGB_TO_XYZ · rgb_cam` when the accessor matrix is
/// finite and non-singular (guarded — §43.2 rule 3 otherwise).
fn derive_cam_to_xyz(rgb_cam: &[[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    const SRGB_TO_XYZ: [[f64; 3]; 3] = [
        [0.412_456_4, 0.357_576_1, 0.180_437_5],
        [0.212_672_9, 0.715_152_2, 0.072_175_0],
        [0.019_333_9, 0.119_192, 0.950_304_1],
    ];
    for row in rgb_cam {
        for v in row {
            if !v.is_finite() {
                return None;
            }
        }
    }
    // An all-zero (uninitialized) accessor answer is "no matrix".
    let mag: f32 = rgb_cam.iter().flatten().map(|v| v.abs()).sum();
    if mag < 1e-6 {
        return None;
    }
    // cam→XYZ = SRGB_TO_XYZ × rgb_cam  (rgb_cam maps camera → sRGB).
    let mut out = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            let mut acc = 0.0f64;
            for k in 0..3 {
                acc += SRGB_TO_XYZ[i][k] * f64::from(rgb_cam[k][j]);
            }
            out[i][j] = acc as f32;
        }
    }
    // Singular check via determinant (f64).
    let m = out;
    let det = f64::from(m[0][0])
        * (f64::from(m[1][1]) * f64::from(m[2][2]) - f64::from(m[1][2]) * f64::from(m[2][1]))
        - f64::from(m[0][1])
            * (f64::from(m[1][0]) * f64::from(m[2][2]) - f64::from(m[1][2]) * f64::from(m[2][0]))
        + f64::from(m[0][2])
            * (f64::from(m[1][0]) * f64::from(m[2][1]) - f64::from(m[1][1]) * f64::from(m[2][0]));
    if det.abs() < 1e-9 {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_matrix_identity_rgb_cam_gives_srgb_to_xyz() {
        let ident = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let m = derive_cam_to_xyz(&ident).expect("identity is usable");
        assert!((m[0][0] - 0.412_456_4).abs() < 1e-6);
        assert!((m[2][2] - 0.950_304_1).abs() < 1e-6);
    }

    #[test]
    fn derive_matrix_refuses_zero_nan_singular() {
        assert!(derive_cam_to_xyz(&[[0.0; 3]; 3]).is_none());
        let mut nan = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        nan[1][1] = f32::NAN;
        assert!(derive_cam_to_xyz(&nan).is_none());
        // Rank-deficient: two equal columns.
        let sing = [[1.0f32, 1.0, 0.0], [0.5, 0.5, 0.0], [0.2, 0.2, 1.0]];
        assert!(derive_cam_to_xyz(&sing).is_none());
    }

    #[test]
    fn module_unavailable_maps_to_raw_disabled() {
        let err = module_unavailable("test detail");
        assert_eq!(err.message_id(), "error.unsupported.raw_disabled");
    }
}
