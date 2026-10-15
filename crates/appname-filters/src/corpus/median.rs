// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `filter.median` — ES §16.3 Median block: per-channel median over the
//! `(2r+1)²` window (premultiplied channels, ALPHA INCLUDED), via the
//! sliding-histogram method [LEGAL: Huang, Yang & Tang 1979 — the ES
//! citation]: stepping one pixel right removes the leaving column and adds
//! the entering one (2r+1 updates each); the median reads by cumulative
//! count. Binning [NORMATIVE for parity]: two-level 256×256 (coarse bin,
//! then refine) — exact to 16-bit quantization (8-bit-origin values land
//! exactly). Param: radius i32 1..=100 def 2. ROI `out.inflate(radius)`;
//! edges clamp; preview LiveProxy; the GPU pair covers r ≤ 4 only
//! (sorting networks) — `gpu_key` notes the fallback rule, the registry's
//! `gpu:false`-above-r-4 row is the O-RENDER handoff. Rights: L0
//! citation-only (1979 publication; select-and-screen judgment, no
//! release claim).
//!
//! v1 pin: window values quantize to 16 bits (`round(v·65535)` clamped)
//! for binning and the RESULT is the exact median of the quantized
//! window, returned as `q/65535` — the §16.3 "exact to 16-bit
//! quantization" contract (f32 documents inherit the 16-bit grid here).

use appname_core::IRect;
use appname_tools::resample::{EdgePolicy, Plane};
use serde_json::Value;

use crate::registry::FilterError;
use crate::traits::{Filter, FilterCtx, FilterGroup, FilterUi, PreviewPolicy};

pub struct Median;

fn radius_of(params: &Value) -> Result<i64, FilterError> {
    match params.get("radius") {
        None => Ok(2),
        Some(v) => {
            let r = v.as_i64().unwrap_or(-1);
            if !(1..=100).contains(&r) {
                return Err(FilterError::ParamOutOfRange {
                    param: "radius",
                    detail: format!("{v} outside 1..=100 (§16.3)"),
                });
            }
            Ok(r)
        }
    }
}

impl Filter for Median {
    fn id(&self) -> &'static str {
        "filter.median"
    }

    fn param_schema(&self) -> &'static str {
        r#"{"$id":"https://appname.org/spec/filter/median.v1.json","type":"object","additionalProperties":false,"properties":{"radius":{"type":"integer","minimum":1,"maximum":100,"default":2,"description":"px (§16.3)"}}}"#
    }

    fn ui_hints(&self) -> FilterUi {
        FilterUi {
            preview: PreviewPolicy::LiveProxy,
            has_dialog: true,
            group: FilterGroup::Noise,
            checkpoint: false,
        }
    }

    fn roi(&self, out: IRect, params: &Value, _ctx: &FilterCtx) -> IRect {
        let r = radius_of(params).unwrap_or(2) as i32;
        out.inflate(r, r)
    }

    fn edge(&self) -> crate::traits::EdgeRule {
        // ES §16.3 Median block: "edges clamp".
        crate::traits::EdgeRule::Clamp
    }

    fn gpu_key(&self) -> &'static str {
        "filter.median" // r ≤ 4 sorting networks; larger r CPU-only (§16.3 registry row)
    }

    fn validate(&self, params: &Value) -> Result<(), FilterError> {
        radius_of(params).map(|_| ())
    }

    fn cpu(
        &self,
        input: &Plane,
        out_rect: IRect,
        params: &Value,
        _ctx: &FilterCtx,
    ) -> Result<Plane, FilterError> {
        if input.rect.intersect(&out_rect) != out_rect {
            return Err(FilterError::RoiNotCovered);
        }
        let r = radius_of(params)? as i32;
        Ok(median_plane(input, out_rect, r))
    }
}

#[inline]
fn q16(v: f32) -> u16 {
    (v * 65535.0).round().clamp(0.0, 65535.0) as u16
}

/// Two-level sliding histogram (coarse 256 × fine 256), per channel.
struct TwoLevel {
    coarse: [u32; 256],
    fine: Vec<u32>, // 256 × 256
    count: u32,
}

impl TwoLevel {
    fn new() -> TwoLevel {
        TwoLevel {
            coarse: [0; 256],
            fine: vec![0; 65536],
            count: 0,
        }
    }

    #[inline]
    fn add(&mut self, q: u16) {
        self.coarse[(q >> 8) as usize] += 1;
        self.fine[q as usize] += 1;
        self.count += 1;
    }

    #[inline]
    fn remove(&mut self, q: u16) {
        self.coarse[(q >> 8) as usize] -= 1;
        self.fine[q as usize] -= 1;
        self.count -= 1;
    }

    /// The k-th smallest (k 1-based = (count+1)/2 for the odd-window
    /// median): coarse scan then refine — the Huang read.
    fn median(&self) -> u16 {
        let k = self.count.div_ceil(2);
        let mut acc = 0u32;
        for c in 0..256usize {
            let n = self.coarse[c];
            if acc + n >= k {
                let mut acc2 = acc;
                for f in 0..256usize {
                    acc2 += self.fine[c * 256 + f];
                    if acc2 >= k {
                        return ((c << 8) | f) as u16;
                    }
                }
            }
            acc += n;
        }
        65535
    }
}

pub fn median_plane(input: &Plane, out_rect: IRect, r: i32) -> Plane {
    let mut out = Plane::new(out_rect);
    // Per row sweep with 4 channel histograms, sliding horizontally
    // (Huang 1979); rows are independent (simple, exact; the per-row
    // rebuild keeps the code obvious — the O(1)-amortized column update
    // is applied within the row).
    for y in out_rect.min.y..out_rect.max.y {
        let mut hists = [
            TwoLevel::new(),
            TwoLevel::new(),
            TwoLevel::new(),
            TwoLevel::new(),
        ];
        // Seed the window for the first column.
        let x0 = out_rect.min.x;
        for wx in (x0 - r)..=(x0 + r) {
            for wy in (y - r)..=(y + r) {
                let v = input.get(wx, wy, EdgePolicy::Clamp);
                for c in 0..4 {
                    hists[c].add(q16(v[c]));
                }
            }
        }
        for x in out_rect.min.x..out_rect.max.x {
            let mut px = [0.0f32; 4];
            for c in 0..4 {
                px[c] = hists[c].median() as f32 / 65535.0;
            }
            out.set(x, y, px);
            // Slide: remove the leaving column, add the entering one.
            if x + 1 < out_rect.max.x {
                for wy in (y - r)..=(y + r) {
                    let leave = input.get(x - r, wy, EdgePolicy::Clamp);
                    let enter = input.get(x + r + 1, wy, EdgePolicy::Clamp);
                    for c in 0..4 {
                        hists[c].remove(q16(leave[c]));
                        hists[c].add(q16(enter[c]));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_matches_the_direct_sort_oracle() {
        let extent = IRect::new(0, 0, 20, 16);
        let mut src = Plane::new(extent);
        for y in 0..16 {
            for x in 0..20 {
                let v = ((x * 37 + y * 23) % 89) as f32 / 88.0;
                src.set(x, y, [v, 1.0 - v, (v * 2.0) % 1.0, 1.0]);
            }
        }
        let r = 2;
        let out = median_plane(&src, extent, r);
        for y in 0..16i32 {
            for x in 0..20i32 {
                for c in 0..4usize {
                    let mut window = Vec::new();
                    for dy in -r..=r {
                        for dx in -r..=r {
                            window.push(q16(src.get(x + dx, y + dy, EdgePolicy::Clamp)[c]));
                        }
                    }
                    window.sort_unstable();
                    let want = window[window.len() / 2] as f32 / 65535.0;
                    let got = out.get(x, y, EdgePolicy::Clamp)[c];
                    assert!((got - want).abs() < 1e-7, "({x},{y})[{c}]: {got} vs {want}");
                }
            }
        }
    }

    #[test]
    fn impulse_noise_is_removed_flat_regions_untouched() {
        let extent = IRect::new(0, 0, 32, 32);
        let mut src = Plane::new(extent);
        src.px.fill([0.4, 0.4, 0.4, 1.0]);
        src.set(16, 16, [1.0, 0.0, 1.0, 1.0]); // salt pixel
        let out = median_plane(&src, extent, 2);
        let c = out.get(16, 16, EdgePolicy::Clamp);
        for ch in 0..3 {
            assert!((c[ch] - 0.4).abs() < 1e-4, "impulse removed: {c:?}");
        }
        let f = out.get(4, 4, EdgePolicy::Clamp);
        assert!((f[0] - 0.4).abs() < 1e-4, "flat untouched");
    }

    #[test]
    fn eight_bit_values_are_exact_through_the_bins() {
        // u8-origin values (k/255) land on exact 16-bit bins (§16.3
        // "8-bit-origin documents … exact"): the median returns one of the
        // input values bit-exactly after the k/255 → q16 → /65535 trip.
        let vals_u8 = [10u8, 20, 30, 200, 240];
        let mut window: Vec<f32> = Vec::new();
        for (i, v) in vals_u8.iter().cycle().take(25).enumerate() {
            let _ = i;
            window.push(*v as f32 / 255.0);
        }
        let extent = IRect::new(0, 0, 5, 5);
        let mut src = Plane::new(extent);
        for y in 0..5 {
            for x in 0..5 {
                let v = window[(y * 5 + x) as usize];
                src.set(x, y, [v, v, v, 1.0]);
            }
        }
        let out = median_plane(&src, extent, 2);
        let got = out.get(2, 2, EdgePolicy::Clamp)[0];
        let mut qs: Vec<u16> = window.iter().map(|v| q16(*v)).collect();
        qs.sort_unstable();
        assert_eq!(q16(got), qs[12], "quantized median identity");
    }

    #[test]
    fn params_and_roi() {
        let f = Median;
        assert!(f.validate(&serde_json::json!({"radius": 100})).is_ok());
        assert!(f.validate(&serde_json::json!({"radius": 101})).is_err());
        assert!(f.validate(&serde_json::json!({"radius": 0})).is_err());
        let ctx = FilterCtx {
            canvas: IRect::new(0, 0, 64, 64),
            seed: 0,
            lod: 0,
        };
        assert_eq!(
            f.roi(
                IRect::new(10, 10, 20, 20),
                &serde_json::json!({"radius": 5}),
                &ctx
            ),
            IRect::new(5, 5, 25, 25)
        );
    }
}
