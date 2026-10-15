// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §17.1 [NORMATIVE] — the `Adjustment` trait (id, param schema,
//! `compile → AdjKernel`), `AdjKernel::{Lut1D, PerPixel}` (`Lut3D` joins
//! with `.cube` §17.3), and the §17.2 evaluation-domain discipline
//! implemented ONCE here: un-premultiply → kernel → re-premultiply;
//! [NORMATIVE] 1024-entry per-channel LUTs on a LINEAR input axis with the
//! display wrapping baked in ("one table lookup either way"), linear
//! interpolation between entries (pin), and the out-of-domain f32 fallback
//! to the closed form — no silent clamp-by-LUT.

use appname_tools::resample::{EdgePolicy, Plane};
use serde_json::Value;
use std::sync::Arc;

/// Per-channel closed form: `f(value, channel_index) -> value` on
/// un-premultiplied working values (the block's own domain wrapping is
/// inside `f`).
pub type ChannelFn = Arc<dyn Fn(f32, usize) -> f32 + Send + Sync>;
/// Cross-channel closed form on un-premultiplied `[r, g, b, a]`.
pub type PixelFn = Arc<dyn Fn([f32; 4]) -> [f32; 4] + Send + Sync>;

/// §17.1 `AdjKernel`, CPU half.
pub enum AdjKernel {
    /// Per-channel 1024-entry LUTs (RGB; alpha always untouched in the
    /// slice-1 corpus) + the closed form for out-of-[0,1] inputs.
    Lut1D {
        tables: [Vec<f32>; 3],
        closed: ChannelFn,
    },
    /// Cross-channel per-pixel function.
    PerPixel(PixelFn),
}

/// §17.1 `Adjustment`.
pub trait Adjustment: Send + Sync {
    /// `adjustment.snake_case`, stable forever.
    fn id(&self) -> &'static str;
    fn param_schema(&self) -> &'static str;
    /// Validate params (ranges + cross-fields).
    fn validate(&self, params: &Value) -> Result<(), AdjError>;

    /// The CONTRACT validation surface (strictness ruling 2026-08-17,
    /// O-UI probe filing; mirror of `Filter::validate_strict`): the
    /// per-op hand validator runs FIRST (precise taxonomy), THEN the
    /// registered schema's structural letter — unknown keys refuse
    /// typed per `additionalProperties: false` at every depth (the
    /// levels tuples and hue/sat bands declare it too),
    /// required/type/enum rows hold — as the backstop. The §12.1 op
    /// bridge calls THIS.
    fn validate_strict(&self, params: &Value) -> Result<(), AdjError> {
        // Hand validator first (precise taxonomy), shape letter second
        // (the strictness backstop) — mirror of Filter::validate_strict.
        self.validate(params)?;
        appname_tools::schema_shape::check_shape(self.param_schema(), params).map_err(|v| {
            AdjError::ParamOutOfRange {
                param: "params",
                detail: format!("{v}"),
            }
        })
    }
    /// Compile to the kernel (§17.1; LUT where applicable — DD ㉘).
    fn compile(&self, params: &Value) -> Result<AdjKernel, AdjError>;
}

/// Param validation failures.
#[derive(Clone, Debug, PartialEq)]
pub enum AdjError {
    ParamOutOfRange { param: &'static str, detail: String },
    ParamMissing(&'static str),
}

impl core::fmt::Display for AdjError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AdjError::ParamOutOfRange { param, detail } => write!(f, "param {param:?}: {detail}"),
            AdjError::ParamMissing(p) => write!(f, "param {p:?} is required"),
        }
    }
}

impl std::error::Error for AdjError {}

/// The ONE interim display-transfer pair (§5 hook; lib.rs divergence note).
#[inline]
pub fn display_encode(v: f32) -> f32 {
    appname_core::io::srgb::srgb_encode(v)
}

#[inline]
pub fn display_decode(v: f32) -> f32 {
    appname_core::io::srgb::srgb_decode(v)
}

/// Bake a display-referred per-channel closed form into the linear-axis
/// 1024 table: `T[i] = decode(f_display(encode(i/1023)))`.
pub fn bake_display_lut(f: impl Fn(f32, usize) -> f32) -> [Vec<f32>; 3] {
    let mut tables: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for (c, table) in tables.iter_mut().enumerate() {
        *table = (0..1024)
            .map(|i| {
                let v = i as f32 / 1023.0;
                display_decode(f(display_encode(v), c))
            })
            .collect();
    }
    tables
}

/// Wrap a display-referred per-channel form as the out-of-domain closed
/// fallback (linear in → linear out).
pub fn display_closed(f: impl Fn(f32, usize) -> f32 + Send + Sync + 'static) -> ChannelFn {
    Arc::new(move |v, c| display_decode(f(display_encode(v), c)))
}

#[inline]
fn lut_sample(table: &[f32], v: f32) -> f32 {
    // Linear interpolation between the 1024 entries (pin).
    let x = v * 1023.0;
    let i = (x.floor() as usize).min(1022);
    let t = x - i as f32;
    table[i] * (1.0 - t) + table[i + 1] * t
}

/// Bookkeeping from an apply pass (the §17.1 info-panel seam).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ApplyStats {
    pub pixels: u64,
    /// Pixels that took the out-of-domain closed-form fallback.
    pub fallback_pixels: u64,
}

/// One pixel through a kernel under the §17.2 evaluation-domain rule
/// (un-premultiply → kernel → re-premultiply; alpha untouched). Returns
/// the new premultiplied pixel and whether the out-of-domain closed-form
/// fallback fired ([NORMATIVE] no silent clamp-by-LUT). Extracted from
/// [`apply_kernel`] for the CP-0.7 `image.auto_*` bake kinds, which apply
/// the same discipline pixel-at-a-time inside the shared selection-gated
/// edit loop.
#[inline]
pub fn apply_kernel_pixel(kernel: &AdjKernel, premul: [f32; 4]) -> ([f32; 4], bool) {
    let s = appname_core::io::srgb::unpremul(premul);
    let mut fallback = false;
    let v = match kernel {
        AdjKernel::Lut1D { tables, closed } => {
            let mut v = s;
            let in_domain = (0..3).all(|c| (0.0..=1.0).contains(&s[c]));
            if in_domain {
                for c in 0..3 {
                    v[c] = lut_sample(&tables[c], s[c]);
                }
            } else {
                // [NORMATIVE] no silent clamp-by-LUT.
                fallback = true;
                for c in 0..3 {
                    v[c] = closed(s[c], c);
                }
            }
            v
        }
        AdjKernel::PerPixel(f) => f(s),
    };
    ([v[0] * s[3], v[1] * s[3], v[2] * s[3], s[3]], fallback)
}

/// Apply a kernel over a plane with the §17.2 evaluation-domain rule.
/// Alpha passes through untouched for every slice-1 kernel.
pub fn apply_kernel(input: &Plane, kernel: &AdjKernel) -> (Plane, ApplyStats) {
    let mut out = Plane::new(input.rect);
    let mut stats = ApplyStats::default();
    for y in input.rect.min.y..input.rect.max.y {
        for x in input.rect.min.x..input.rect.max.x {
            let p = input.get(x, y, EdgePolicy::Transparent);
            stats.pixels += 1;
            let (v, fallback) = apply_kernel_pixel(kernel, p);
            if fallback {
                stats.fallback_pixels += 1;
            }
            out.set(x, y, v);
        }
    }
    (out, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use appname_core::IRect;

    #[test]
    fn lut_identity_round_trip_is_tight() {
        // Baking the identity display form yields ≈identity linear tables.
        let tables = bake_display_lut(|v, _| v);
        for table in tables.iter().take(3) {
            for i in (0..1024).step_by(51) {
                let v = i as f32 / 1023.0;
                assert!((lut_sample(table, v) - v).abs() < 2e-3, "i={i}");
            }
        }
    }

    #[test]
    fn out_of_domain_takes_the_closed_form_not_the_lut() {
        let kernel = AdjKernel::Lut1D {
            tables: bake_display_lut(|v, _| v),
            closed: Arc::new(|v, _| v * 2.0), // deliberately different
        };
        let mut p = Plane::new(IRect::new(0, 0, 2, 1));
        p.set(0, 0, [0.5, 0.5, 0.5, 1.0]); // in-domain
        p.set(1, 0, [3.0, 3.0, 3.0, 1.0]); // HDR out-of-domain
        let (out, stats) = apply_kernel(&p, &kernel);
        assert!(
            (out.get(0, 0, EdgePolicy::Transparent)[0] - 0.5).abs() < 2e-3,
            "LUT path"
        );
        assert!(
            (out.get(1, 0, EdgePolicy::Transparent)[0] - 6.0).abs() < 1e-5,
            "closed path"
        );
        assert_eq!(stats.fallback_pixels, 1, "§17.2 fallback accounted");
    }

    #[test]
    fn evaluation_domain_unpremultiplies_and_restores() {
        // in = 50% gray at α = 0.5 (premul 0.25): an invert-ish kernel sees
        // STRAIGHT 0.5; output re-premultiplies.
        let kernel = AdjKernel::PerPixel(Arc::new(|s| [1.0 - s[0], 1.0 - s[1], 1.0 - s[2], s[3]]));
        let mut p = Plane::new(IRect::new(0, 0, 1, 1));
        p.set(0, 0, [0.25, 0.25, 0.25, 0.5]);
        let (out, _) = apply_kernel(&p, &kernel);
        let v = out.get(0, 0, EdgePolicy::Transparent);
        assert!(
            (v[0] - 0.25).abs() < 1e-6,
            "straight 0.5 → 0.5 → premul 0.25"
        );
        assert_eq!(v[3], 0.5, "alpha untouched");
    }
}
