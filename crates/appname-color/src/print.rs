// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §29.3 (color-managed print — pipeline step 4: "convert
//! working → printer/paper profile with the chosen rendering intent +
//! BPC (§5.2); the dialog preview *is* the soft proof (§5.6) including
//! gamut warning"; step 3 [PERF]: strip composition, "each strip
//! ≤ 256 MiB (`strip_rows = floor(256 MiB / (paper_width_px × 16 B))`
//! at RGBA f32 working precision)"), §5.6 (the preview chain), §5.2
//! (`NOCACHE` transforms — strip splitting can never change output
//! bytes); DD ㉞.
//!
//! Print pipeline v1, color-side. Page setup, scaling, marks geometry,
//! spool composition, and the platform submit (steps 1–2, 5–6) are
//! shell/IO territory; what is color's is **step 4** — the working →
//! printer conversion for each composed strip — and the §5.6 tie-in that
//! makes the print dialog's preview the soft proof for the same
//! profile + intent + BPC. Both are thin, named integration points over the
//! CP-1.0 CMS machinery (CMYK printer profiles ride the `icc.rs`
//! CMYK float marshalling; RGB printers ride the RGB paths).

use std::sync::Arc;

use crate::error::ColorError;
use crate::profile::{DataColorSpace, ProfileUid};
use crate::proof::SoftProof;
use crate::transform::{Cms, Transform, TransformKey};
use crate::types::{ColorModel, Depth, Intent, PixelFormat};

/// §29.3 [PERF]: per-strip memory budget, 256 MiB.
pub const STRIP_BUDGET_BYTES: u64 = 256 * 1024 * 1024;

/// §29.3: RGBA f32 working precision, 16 bytes per pixel.
pub const WORKING_BYTES_PER_PX: u64 = 16;

/// §29.3 step 3: `strip_rows = floor(256 MiB / (paper_width_px × 16 B))`,
/// floored at 1 row so pathological widths still make progress.
pub fn strip_rows(paper_width_px: u32) -> u32 {
    if paper_width_px == 0 {
        return 1;
    }
    let rows = STRIP_BUDGET_BYTES / (u64::from(paper_width_px) * WORKING_BYTES_PER_PX);
    u32::try_from(rows.max(1)).unwrap_or(u32::MAX)
}

/// The §29.3 print color parameters: printer/paper profile + rendering
/// intent + black-point compensation (the dialog's color block).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PrintColorParams {
    pub printer_profile: ProfileUid,
    pub intent: Intent,
    pub bpc: bool,
}

impl PrintColorParams {
    /// §29.3: "the dialog preview *is* the soft proof (§5.6) including
    /// gamut warning" — the `SoftProof` for this print job, same
    /// profile + intent + BPC. `simulate_paper` is the dialog's paper-white
    /// toggle (§5.6 absolute-colorimetric leg).
    pub const fn soft_proof(&self, simulate_paper: bool, gamut_warning: bool) -> SoftProof {
        SoftProof {
            proof_profile: self.printer_profile,
            intent: self.intent,
            bpc: self.bpc,
            simulate_paper,
            gamut_warning,
        }
    }
}

/// Resolve the printer-side buffer model (alpha-bearing: composed strips
/// are opaque RGBA working pixels and alpha rides through `COPY_ALPHA`
/// untouched; CMYK printers get `CmykA`, RGB printers `Rgba`, gray
/// printers `GrayA`).
fn printer_model(cms: &Cms, params: &PrintColorParams) -> Result<ColorModel, ColorError> {
    Ok(
        match cms.registry().get(params.printer_profile)?.data_space {
            DataColorSpace::Rgb => ColorModel::Rgba,
            DataColorSpace::Gray => ColorModel::GrayA,
            DataColorSpace::Cmyk => ColorModel::CmykA,
            DataColorSpace::Lab => ColorModel::LabA,
            _ => return Err(ColorError::ProfileSpaceMismatch),
        },
    )
}

/// §29.3 step 4: the working → printer transform for strip conversion —
/// RGBA f32 working in, the printer profile's model (+alpha) out, the
/// job's intent + BPC. Cached like every transform (§5.4); `NOCACHE`
/// makes the strip split irrelevant to output bytes.
pub fn print_transform(
    cms: &mut Cms,
    working: ProfileUid,
    params: &PrintColorParams,
) -> Result<Arc<Transform>, ColorError> {
    let out_model = printer_model(cms, params)?;
    cms.transform(TransformKey {
        src: working,
        dst: params.printer_profile,
        proof: None,
        intent: params.intent,
        bpc: params.bpc,
        fmt_in: PixelFormat::new(ColorModel::Rgba, Depth::F32),
        fmt_out: PixelFormat::new(out_model, Depth::F32),
    })
}

/// Convert one composed strip (interleaved RGBA f32 working, straight
/// alpha — the §5.2 premultiplied rule is the composer's to honor before
/// this call). Thin over [`Transform::apply_f32`]; named so the spool
/// loop reads as the §29.3 pipeline.
pub fn convert_print_strip(
    t: &Transform,
    strip_working: &[f32],
    strip_out: &mut [f32],
) -> Result<(), ColorError> {
    t.apply_f32(strip_working, strip_out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::BuiltinSpace;

    /// The §29.3 formula, hand-evaluated: a 3600-px-wide letter sheet at
    /// 300 dpi → floor(268435456 / 57600) = 4660 rows; poster widths
    /// still ≥ 1.
    #[test]
    fn strip_rows_formula() {
        assert_eq!(strip_rows(3600), 268_435_456 / (3600 * 16));
        assert_eq!(strip_rows(3600), 4660);
        // A 17 M-px-wide pathological sheet still makes progress.
        assert_eq!(strip_rows(u32::MAX), 1);
        assert_eq!(strip_rows(0), 1);
        // Budget honored: rows × width × 16 ≤ 256 MiB.
        for w in [800u32, 3600, 14_400, 100_000] {
            let r = u64::from(strip_rows(w));
            assert!(r * u64::from(w) * WORKING_BYTES_PER_PX <= STRIP_BUDGET_BYTES);
        }
    }

    /// The dialog preview IS the soft proof: same profile, intent, bpc.
    #[test]
    fn print_preview_is_the_soft_proof() {
        let params = PrintColorParams {
            printer_profile: crate::profile::ProfileUid::of_builtin(BuiltinSpace::Srgb),
            intent: Intent::Perceptual,
            bpc: true,
        };
        let sp = params.soft_proof(true, true);
        assert_eq!(sp.proof_profile, params.printer_profile);
        assert_eq!(sp.intent, Intent::Perceptual);
        assert!(sp.bpc && sp.simulate_paper && sp.gamut_warning);
    }

    /// Step 4 end-to-end on an RGB "printer" (sRGB bytes): the transform
    /// builds, converts a strip, and — the §5.2 NOCACHE guarantee — the
    /// same pixels split into different strip sizes produce byte-identical
    /// output.
    #[test]
    fn strip_split_cannot_change_bytes() {
        let mut cms = Cms::new();
        let printer = cms
            .open_profile(lcms2::Profile::new_srgb().icc().unwrap(), ColorModel::Rgba)
            .unwrap();
        let params = PrintColorParams {
            printer_profile: printer,
            intent: Intent::RelativeColorimetric,
            bpc: true,
        };
        let working = cms.builtin(BuiltinSpace::DisplayP3);
        let t = print_transform(&mut cms, working, &params).unwrap();
        // A seeded 30-px "page", 4 channels.
        let mut state: u64 = 0x9317;
        let page: Vec<f32> = (0..120)
            .map(|_| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (((state >> 11) as f64) / ((1u64 << 53) as f64)) as f32
            })
            .collect();
        let mut whole = vec![0f32; page.len()];
        convert_print_strip(&t, &page, &mut whole).unwrap();
        // Split at an awkward pixel boundary (7 px + 23 px).
        let mut split = vec![0f32; page.len()];
        let cut = 7 * 4;
        convert_print_strip(&t, &page[..cut], &mut split[..cut]).unwrap();
        convert_print_strip(&t, &page[cut..], &mut split[cut..]).unwrap();
        for (a, b) in whole.iter().zip(split.iter()) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }
}
