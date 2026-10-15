// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §17.2 "Auto Tone, Auto Contrast, Auto Color" [NORMATIVE]:
//! histogram analyses that emit ordinary **Levels params** — re-editable,
//! zero hidden state; full-res histogram, force-recounted if stale (the
//! staleness rule is the evaluator's; these are the pure analyses).
//!
//! ```text
//! clip(hist, fLo, fHi):  lo = smallest bin with cumsum          ≥ fLo·total
//!                        hi = largest  bin with cumsum-from-top ≥ fHi·total
//! ```
//!
//! * Auto Tone — per channel independently: `inBlack_c = lo_c/255`,
//!   `inWhite_c = hi_c/255`, gamma 1 (full-range stretch; may shift color
//!   balance — that is its point).
//! * Auto Contrast — ONE `(lo, hi)` from the LUMA histogram, written
//!   identically to all three channels (cast preserved).
//! * Auto Color — per-channel clip as Auto Tone, plus midtone
//!   neutralization: `m_c` = mean of channel `c` (display-encoded,
//!   un-premultiplied — the histogram domain) over pixels with luma bin ∈
//!   [110, 145]; after the black/white remap of that mean (`m_c′`), solve
//!   the gamma that sends the mean to 0.5 — "the midtone mean maps to
//!   0.5, killing the cast" [NORMATIVE defining property].
//!
//!   SPEC-CONFLICT PIN (lane log): §17.2 prints `gamma_c =
//!   ln(0.5)/ln(clamp(m_c′, 0.05, 0.95))`, but under the §17.2 levels
//!   convention `out = n^(1/gamma)` that expression maps the mean to
//!   `m′^(ln m′/ln 0.5) ≠ 0.5` — the printed ratio only satisfies the
//!   block's own defining sentence with `out = n^gamma`. The defining
//!   property wins here: `gamma_c = ln(clamp(m_c′, .05, .95))/ln(0.5)`
//!   (the same clamp), so `m′^(1/γ) = 0.5` exactly — tested. Flagged as a
//!   spec question; a fold-in ruling flips one line either way.
//!
//!   SPEC-CONFLICT PIN #2 (T1.7 hardening round): §17.2's clamp of `m_c′`
//!   to `[0.05, 0.95]` and the Levels `gamma` range `0.1–10` (ES:7438)
//!   are mutually INCONSISTENT at the bright end. The defining property
//!   needs `gamma = ln(m′)/ln(0.5)`, which reaches the 0.1 floor at
//!   `m′ = 0.5^0.1 = 0.9330330`; for `m′ ∈ (0.9330330, 0.95]` NO legal
//!   gamma sends the mean to 0.5, and at the `0.95` clamp the mean lands
//!   at `0.95^10 = 0.5987`. This code keeps the LEGAL Levels range —
//!   emitting `gamma < 0.1` would be a param `adjustment.levels`'s own
//!   parser rejects — and accepts the shortfall. Reachable in practice on
//!   a strongly cast channel. Pinned by
//!   `auto_color_bright_end_hits_the_gamma_floor_and_cannot_reach_half`;
//!   flagged for fold-in (the dark end has no such conflict).
//!
//! Defaults per the §17.2 prefs + the §10.3 `image.auto_*` rows:
//! `clipLow = clipHigh = 0.001` (fractions). The `image.auto_*` bake kinds
//! in `kinds.rs` consume these analyses (they are OpNode-independent); the
//! wiring slice LANDED at 03b7fd5 — this note previously said it was still
//! pending, corrected in the T1.7 hardening round.

use appname_tools::resample::{EdgePolicy, Plane};

use crate::histogram::{bin_of, Histogram};
use crate::kernel::display_encode;
use crate::ops::LevelsTuple;

/// The §17.2 clip scan over one 256-bin plane.
pub fn clip(bins: &[u32; 256], total: u64, f_lo: f64, f_hi: f64) -> (u8, u8) {
    let need_lo = (f_lo * total as f64).max(0.0);
    let need_hi = (f_hi * total as f64).max(0.0);
    let mut lo = 0u8;
    let mut cum = 0u64;
    for (b, &bin) in bins.iter().enumerate() {
        cum += bin as u64;
        if cum as f64 >= need_lo {
            lo = b as u8;
            break;
        }
    }
    let mut hi = 255u8;
    let mut cum_top = 0u64;
    for b in (0..256usize).rev() {
        cum_top += bins[b] as u64;
        if cum_top as f64 >= need_hi {
            hi = b as u8;
            break;
        }
    }
    (lo, hi.max(lo)) // degenerate histograms collapse to lo = hi
}

/// Per-channel Levels params (r/g/b tuples) — the §17.2 emission shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoLevels {
    pub r: LevelsTuple,
    pub g: LevelsTuple,
    pub b: LevelsTuple,
}

fn stretch(lo: u8, hi: u8) -> LevelsTuple {
    let (mut ib, mut iw) = (lo as f32 / 255.0, hi as f32 / 255.0);
    // Keep the §17.2 levels gap invariant (≥ 1/255) on degenerate input.
    if iw - ib < 1.0 / 255.0 {
        iw = (ib + 1.0 / 255.0).min(1.0);
        ib = iw - 1.0 / 255.0;
    }
    LevelsTuple {
        in_black: ib,
        in_white: iw,
        gamma: 1.0,
        out_black: 0.0,
        out_white: 1.0,
    }
}

/// §17.2 Auto Tone.
pub fn auto_tone(hist: &Histogram, f_lo: f64, f_hi: f64) -> AutoLevels {
    let t = |c: usize| {
        let (lo, hi) = clip(&hist.bins[c], hist.total, f_lo, f_hi);
        stretch(lo, hi)
    };
    AutoLevels {
        r: t(0),
        g: t(1),
        b: t(2),
    }
}

/// §17.2 Auto Contrast: one luma clip for all channels.
pub fn auto_contrast(hist: &Histogram, f_lo: f64, f_hi: f64) -> AutoLevels {
    let (lo, hi) = clip(&hist.bins[3], hist.total, f_lo, f_hi);
    let t = stretch(lo, hi);
    AutoLevels { r: t, g: t, b: t }
}

/// The §17.2 Auto Color midtone means: per channel, the mean of the
/// display-encoded un-premultiplied value over pixels whose LUMA bin lies
/// in [110, 145]. `None` when no pixel qualifies.
pub fn midtone_means(plane: &Plane) -> Option<[f32; 3]> {
    let mut sums = [0.0f64; 3];
    let mut n = 0u64;
    for y in plane.rect.min.y..plane.rect.max.y {
        for x in plane.rect.min.x..plane.rect.max.x {
            let p = plane.get(x, y, EdgePolicy::Transparent);
            if p[3] == 0.0 {
                continue;
            }
            let s = appname_core::io::srgb::unpremul(p);
            let enc = [
                display_encode(s[0]),
                display_encode(s[1]),
                display_encode(s[2]),
            ];
            let luma_bin = bin_of(0.3 * enc[0] + 0.59 * enc[1] + 0.11 * enc[2]);
            if (110..=145).contains(&luma_bin) {
                for c in 0..3 {
                    sums[c] += enc[c] as f64;
                }
                n += 1;
            }
        }
    }
    if n == 0 {
        return None;
    }
    Some([
        (sums[0] / n as f64) as f32,
        (sums[1] / n as f64) as f32,
        (sums[2] / n as f64) as f32,
    ])
}

/// §17.2 Auto Color: per-channel clip + optional midtone neutralization.
pub fn auto_color(
    hist: &Histogram,
    midtones: Option<[f32; 3]>,
    f_lo: f64,
    f_hi: f64,
    neutralize: bool,
) -> AutoLevels {
    let mut out = auto_tone(hist, f_lo, f_hi);
    if neutralize {
        if let Some(m) = midtones {
            for (c, t) in [&mut out.r, &mut out.g, &mut out.b].into_iter().enumerate() {
                // Remap the mean through the fresh black/white points, then
                // solve the gamma that sends it to 0.5.
                let m_prime = ((m[c] - t.in_black) / (t.in_white - t.in_black)).clamp(0.05, 0.95);
                // Defining-property direction (module-doc conflict pin).
                t.gamma = (m_prime.ln() / 0.5f32.ln()).clamp(0.1, 10.0);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use appname_core::IRect;

    fn plane_of_grays(values_255: &[(u8, usize)]) -> Plane {
        // A row plane with `count` pixels at each encoded gray value.
        let total: usize = values_255.iter().map(|(_, n)| n).sum();
        let mut p = Plane::new(IRect::new(0, 0, total as i32, 1));
        let mut x = 0;
        for &(v, n) in values_255 {
            let lin = appname_core::io::srgb::srgb_decode(v as f32 / 255.0);
            for _ in 0..n {
                p.set(x, 0, [lin, lin, lin, 1.0]);
                x += 1;
            }
        }
        p
    }

    #[test]
    fn clip_scan_matches_the_formula_exactly() {
        // 1000 px: 1 at bin 10, 1 at bin 240, 998 at bin 128.
        let p = plane_of_grays(&[(10, 1), (128, 998), (240, 1)]);
        let h = Histogram::of_plane(&p);
        assert_eq!(h.total, 1000);
        // fLo = 0.001 → need cumsum ≥ 1 → lo = 10; same from the top.
        let (lo, hi) = clip(&h.bins[0], h.total, 0.001, 0.001);
        assert_eq!((lo, hi), (10, 240));
        // Larger clip fractions swallow the outliers.
        let (lo, hi) = clip(&h.bins[0], h.total, 0.002, 0.002);
        assert_eq!((lo, hi), (128, 128), "outliers clipped away");
        // f = 0 is degenerate by the formula's own text (cumsum ≥ 0 holds
        // at the first bin scanned): lo = 0, hi = 255 — pinned.
        let (lo, hi) = clip(&h.bins[0], h.total, 0.0, 0.0);
        assert_eq!((lo, hi), (0, 255));
    }

    #[test]
    fn auto_tone_stretches_per_channel_independently() {
        // Red confined to [50, 200]; green to [80, 120]; blue full-range.
        let mut p = Plane::new(IRect::new(0, 0, 256, 1));
        for x in 0..256i32 {
            let r = appname_core::io::srgb::srgb_decode((50 + (x * 150 / 255)) as f32 / 255.0);
            let g = appname_core::io::srgb::srgb_decode((80 + (x * 40 / 255)) as f32 / 255.0);
            let b = appname_core::io::srgb::srgb_decode(x as f32 / 255.0);
            p.set(x, 0, [r, g, b, 1.0]);
        }
        let h = Histogram::of_plane(&p);
        let al = auto_tone(&h, 0.0005, 0.0005);
        assert_eq!(al.r.in_black, 50.0 / 255.0);
        assert_eq!(al.r.in_white, 200.0 / 255.0);
        assert_eq!(al.g.in_black, 80.0 / 255.0);
        assert_eq!(al.g.in_white, 120.0 / 255.0);
        assert_eq!(al.b.in_black, 0.0);
        assert_eq!(al.b.in_white, 1.0);
        assert!(al.r.gamma == 1.0 && al.g.gamma == 1.0);
        // Auto Contrast: ONE luma pair on all three channels.
        let ac = auto_contrast(&h, 0.0005, 0.0005);
        assert_eq!(ac.r, ac.g);
        assert_eq!(ac.g, ac.b);
    }

    #[test]
    fn auto_color_neutralizes_the_midtone_cast() {
        // A red-cast midtone field: R encoded 150, G/B encoded 120 — luma
        // = .3·150 + .59·120 + .11·120 = 129 → inside [110, 145].
        let mut p = Plane::new(IRect::new(0, 0, 64, 1));
        for x in 0..64 {
            let r = appname_core::io::srgb::srgb_decode(150.0 / 255.0);
            let gb = appname_core::io::srgb::srgb_decode(120.0 / 255.0);
            p.set(x, 0, [r, gb, gb, 1.0]);
        }
        let h = Histogram::of_plane(&p);
        let m = midtone_means(&p).expect("midtones present");
        assert!((m[0] - 150.0 / 255.0).abs() < 1e-3 && (m[1] - 120.0 / 255.0).abs() < 1e-3);
        let al = auto_color(&h, Some(m), 0.001, 0.001, true);
        // The cast channel gets the LOWER gamma? m'_r > m'_g: with clip at
        // the raw extremes both remaps hit the 0.05/0.95 clamp edges …
        // verify the defining property instead: each channel's remapped
        // mean maps to 0.5 through its gamma.
        for (c, t) in [al.r, al.g, al.b].into_iter().enumerate() {
            let m_prime = ((m[c] - t.in_black) / (t.in_white - t.in_black)).clamp(0.05, 0.95);
            let mapped = m_prime.powf(1.0 / t.gamma);
            assert!((mapped - 0.5).abs() < 1e-4, "channel {c}: {mapped}");
        }
        // neutralize = false leaves gamma 1 (plain per-channel stretch).
        let plain = auto_color(&h, Some(m), 0.001, 0.001, false);
        assert_eq!(plain.r.gamma, 1.0);
        // No midtone pixels → gammas stay 1 (None path).
        let dark = plane_of_grays(&[(10, 50)]);
        assert!(midtone_means(&dark).is_none());
    }

    #[test]
    fn degenerate_histograms_keep_the_levels_gap() {
        // Single-value image: lo = hi; the emitted tuple must stay valid
        // for the §17.2 levels gap (≥ 1/255).
        let p = plane_of_grays(&[(128, 100)]);
        let h = Histogram::of_plane(&p);
        let al = auto_tone(&h, 0.001, 0.001);
        assert!(al.r.in_white - al.r.in_black >= 1.0 / 255.0 - 1e-6);
    }

    /// The two RAIL-END degenerate channels — the branch `stretch`'s
    /// `.min(1.0)` clamp exists for, which the mid-gray case above never
    /// reaches. Assertions are the §17.2 envelope itself (range 0..1 +
    /// `gap ≥ 1/255`, ES:7437) plus the physical property that a channel
    /// with no headroom cannot be stretched into any: NOT fitted to output.
    #[test]
    fn rail_end_channels_stay_inside_the_gap_rule_and_cannot_stretch() {
        // Fully SATURATED channel: every pixel in bin 255 ⇒ lo = hi = 255.
        // Naive `iw = ib + 1/255` would leave inWhite = 256/255 > 1, out of
        // the §17.2 0..1 range; the clamp must push the window DOWN instead.
        let sat = plane_of_grays(&[(255, 100)]);
        let hs = Histogram::of_plane(&sat);
        assert_eq!(clip(&hs.bins[0], hs.total, 0.001, 0.001), (255, 255));
        let t = auto_tone(&hs, 0.001, 0.001).r;
        assert!(
            (0.0..=1.0).contains(&t.in_black) && (0.0..=1.0).contains(&t.in_white),
            "§17.2 range 0..1, got {t:?}"
        );
        assert!(
            t.in_white - t.in_black >= 1.0 / 255.0 - 1e-6,
            "§17.2 gap, got {t:?}"
        );
        // No headroom above white: the saturated field must stay saturated.
        assert!((t.eval(1.0) - 1.0).abs() < 1e-6, "white survives: {t:?}");

        // Fully EMPTY channel: every pixel in bin 0 ⇒ lo = hi = 0. The
        // window opens UPWARD here (inBlack pinned at the 0 rail).
        let empty = plane_of_grays(&[(0, 100)]);
        let he = Histogram::of_plane(&empty);
        assert_eq!(clip(&he.bins[0], he.total, 0.001, 0.001), (0, 0));
        let t = auto_tone(&he, 0.001, 0.001).r;
        assert!(
            (0.0..=1.0).contains(&t.in_black) && (0.0..=1.0).contains(&t.in_white),
            "§17.2 range 0..1, got {t:?}"
        );
        assert!(
            t.in_white - t.in_black >= 1.0 / 255.0 - 1e-6,
            "§17.2 gap, got {t:?}"
        );
        // No headroom below black: the empty field must stay empty.
        assert!((t.eval(0.0) - 0.0).abs() < 1e-6, "black survives: {t:?}");
    }

    /// The clip threshold is `≥ fLo·total`, and `fLo·total` is NOT always
    /// an integer. `clip_scan_matches_the_formula_exactly` pins the
    /// integer side (need = 1.0 exactly, which discriminates `≥` from
    /// `>`); this pins the FRACTIONAL side, where one pixel is short of
    /// the threshold and the scan must walk on to the next populated bin.
    #[test]
    fn fractional_clip_threshold_needs_the_next_whole_pixel() {
        // 1500 px, fLo = 0.001 ⇒ need = 1.5 (hand-computed). Bin 10 holds
        // ONE pixel: cumsum 1 < 1.5, so bin 10 does NOT satisfy it. Bin 20
        // brings cumsum to 2 ≥ 1.5 ⇒ lo = 20, not 10.
        let p = plane_of_grays(&[(10, 1), (20, 1), (128, 1498)]);
        let h = Histogram::of_plane(&p);
        assert_eq!(h.total, 1500);
        let (lo, _) = clip(&h.bins[0], h.total, 0.001, 0.0);
        assert_eq!(lo, 20, "one pixel is short of the 1.5-pixel threshold");
    }

    /// The two ends of the §17.2 scan are INDEPENDENT: `lo` is driven by
    /// `fLo·total` from the bottom, `hi` by `fHi·total` from the top.
    /// Every other clip test in this module passes `fLo == fHi`, so a
    /// transposed pair would be invisible to all of them. Fixture and
    /// expectations are hand-computed from the formula text alone.
    #[test]
    fn clip_ends_read_their_own_fraction_and_do_not_transpose() {
        // 2000 px: bin 5 ×1, bin 9 ×2, bin 200 ×1, bin 250 ×3, bin 128
        // ×1993.  fLo = 0.001 ⇒ need_lo = 2.0; fHi = 0.002 ⇒ need_hi = 4.0.
        //   bottom: bin5 cum 1 < 2 → bin9 cum 3 ≥ 2  ⇒ lo = 9
        //   top:    bin250 cum 3 < 4 → bin200 cum 4 ≥ 4 ⇒ hi = 200
        let p = plane_of_grays(&[(5, 1), (9, 2), (128, 1993), (200, 1), (250, 3)]);
        let h = Histogram::of_plane(&p);
        assert_eq!(h.total, 2000);
        assert_eq!(clip(&h.bins[0], h.total, 0.001, 0.002), (9, 200));
        // The TRANSPOSED pair lands somewhere else entirely on BOTH ends —
        // need_lo = 4.0 walks past the outliers into the bulk (lo = 128),
        // need_hi = 2.0 stops at the very first populated bin (hi = 250).
        // So the assertion above discriminates the two orderings.
        assert_eq!(clip(&h.bins[0], h.total, 0.002, 0.001), (128, 250));
    }

    /// A ONE-PIXEL input — the smallest histogram that exists. `need` is
    /// 0.001 px, satisfied by the single pixel at its own bin, so the scan
    /// collapses to `lo = hi = that bin` and the §17.2 gap rule (ES:7437)
    /// must open the window around it.
    #[test]
    fn one_pixel_input_collapses_to_its_own_bin_and_opens_the_gap() {
        let p = plane_of_grays(&[(128, 1)]);
        let h = Histogram::of_plane(&p);
        assert_eq!(h.total, 1);
        assert_eq!(clip(&h.bins[0], h.total, 0.001, 0.001), (128, 128));
        let t = auto_tone(&h, 0.001, 0.001).r;
        assert!(
            (0.0..=1.0).contains(&t.in_black) && (0.0..=1.0).contains(&t.in_white),
            "§17.2 range 0..1, got {t:?}"
        );
        assert!(
            t.in_white - t.in_black >= 1.0 / 255.0 - 1e-6,
            "§17.2 gap, got {t:?}"
        );
        // A one-pixel field is its own black AND white point: the window
        // straddles it, so the pixel itself must still land inside 0..1.
        let v = t.eval(128.0 / 255.0);
        assert!((0.0..=1.0).contains(&v), "one-pixel remap in range: {v}");
    }

    /// SPEC-CONFLICT PIN #2 (new; distinct from the module-doc gamma
    /// direction pin). §17.2 clamps the remapped midtone mean to
    /// `[0.05, 0.95]`, but the Levels `gamma` param's own normative range
    /// is `0.1–10` (ES:7438). Those two bounds are mutually INCONSISTENT
    /// at the bright end: the defining property needs
    /// `gamma = ln(m′)/ln(0.5)`, which reaches the 0.1 floor at
    /// `m′ = 0.5^0.1 = 0.9330330` (hand-computed) — so for
    /// `m′ ∈ (0.9330330, 0.95]` NO legal gamma sends the mean to 0.5.
    ///
    /// This implementation keeps the LEGAL Levels range (emitting
    /// `gamma < 0.1` would be a param the `adjustment.levels` parser
    /// itself rejects), and the mean consequently lands above 0.5. The
    /// expected values below are computed from the formula, not observed:
    /// at the clamp `m′ = 0.95`, `mapped = 0.95^(1/0.1) = 0.95^10`.
    #[test]
    fn auto_color_bright_end_hits_the_gamma_floor_and_cannot_reach_half() {
        // Full-range histogram ⇒ inBlack = 0, inWhite = 1 ⇒ m′ = m_c, so
        // the remap is the identity and the clamp arithmetic is isolated.
        let full = plane_of_grays(&[(0, 1), (128, 998), (255, 1)]);
        let h = Histogram::of_plane(&full);
        let t = auto_tone(&h, 0.0, 0.0).r;
        assert_eq!((t.in_black, t.in_white), (0.0, 1.0), "identity window");

        // JUST BELOW the threshold: a legal gamma exists, so the §17.2
        // defining property holds exactly.
        let below = auto_color(&h, Some([0.90, 0.5, 0.5]), 0.0, 0.0, true);
        let expect = 0.90f32.ln() / 0.5f32.ln(); // = 0.152003, inside 0.1–10
        assert!(
            (below.r.gamma - expect).abs() < 1e-5,
            "unclamped gamma, got {:?}",
            below.r.gamma
        );
        assert!(
            (0.90f32.powf(1.0 / below.r.gamma) - 0.5).abs() < 1e-5,
            "mean maps to 0.5 while the gamma is legal"
        );

        // ABOVE the threshold (and past the m′ clamp): gamma pins at the
        // ES:7438 floor and the mean CANNOT reach 0.5.
        let above = auto_color(&h, Some([0.99, 0.5, 0.5]), 0.0, 0.0, true);
        assert_eq!(above.r.gamma, 0.1, "gamma pinned at the ES:7438 floor");
        let mapped = 0.95f32.powi(10); // m′ clamped to 0.95, then ^(1/0.1)
        assert!(
            (mapped - 0.598_737).abs() < 1e-4,
            "hand-computed 0.95^10, got {mapped}"
        );
        assert!(
            (0.95f32.powf(1.0 / above.r.gamma) - mapped).abs() < 1e-5,
            "the emitted gamma reproduces the hand-computed landing value"
        );
        assert!(mapped - 0.5 > 0.09, "the shortfall is real, not rounding");

        // The DARK end has no such conflict: gamma at the m′ = 0.05 clamp
        // is ln(0.05)/ln(0.5) = 4.321928, comfortably inside 0.1–10.
        let dark = auto_color(&h, Some([0.01, 0.5, 0.5]), 0.0, 0.0, true);
        assert!(
            (dark.r.gamma - 4.321_928).abs() < 1e-4,
            "dark clamp stays legal, got {:?}",
            dark.r.gamma
        );
        assert!(
            (0.05f32.powf(1.0 / dark.r.gamma) - 0.5).abs() < 1e-5,
            "defining property still exact at the dark clamp"
        );
    }
}
