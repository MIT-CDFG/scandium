// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §14.1 (the channel model; "Colorized channel view
//! [NORMATIVE]"; the rubylith overlay for alpha-channels-with-composite,
//! "the same renderer as Quick Mask (§13.4)"), §14.2 (saved selections —
//! the 8-bit lossless-or-not-at-all persistence predicate), §14.3 (spot
//! channels: Post-1.0, but "the data model reserves room" in v1.0 —
//! `ink`/`solidity` display props ship now), §13.4 (Quick Mask rubylith:
//! "renders unselected area as a rubylith tint (default red at 50%)"),
//! §6.2 [NORMATIVE] ("Masks and channels are never converted — they are
//! always Gray"; a spot channel's *preview ink color* is converted for
//! display, its data untouched); plan §2.5 Channels row (O-COLOR side).
//!
//! Color-side channels groundwork. The channel STORAGE (the `ChannelSet`
//! arena, `NamedChannel` COW tile buffers, the §14.2 Arc-sharing
//! round-trip) is core's document graph — O-CORE's fence, doors filed in
//! the lane log. What is color's is the display/persistence MATH:
//! * the §14.1 colorized-component view (values → hues, multi-solo sums),
//! * the §14.1/§13.4 rubylith overlay (ink/solidity, defaults red / 0.5),
//! * the §14.2 8-bit exact-representability predicate (`v = n/255`) that
//!   gates the lossless 8-bit channel encoding in `.appname`,
//! * the §14.3 spot display-prop vocabulary (reserved-room groundwork).
//!
//! Channels are coverage/ink data, NEVER color-managed (§6.2): nothing in
//! this module touches the CMS. The colorized view's OUTPUT is ordinary
//! working-space RGB and is "pushed through the normal §5 display
//! transform" by the renderer afterwards (§14.1).
//!
//! ## Pinned elections (documented for the fold-in register)
//! §14.1 names the Lab opponent-axis ramps but tabulates no pole colors.
//! Election: `a` ramps green → magenta, `b` ramps blue → yellow (the CIE
//! opponent pairs), as working-RGB primaries/secondaries, lerped linearly
//! in `v`. Recorded in the lane log for the v5.6 fold-in list.

/// §14.1 / §13.4 default rubylith ink (red) — `channel.edit` defaults.
pub const RUBYLITH_DEFAULT_INK: [f32; 3] = [1.0, 0.0, 0.0];
/// §14.1 / §13.4 default rubylith solidity (50%).
pub const RUBYLITH_DEFAULT_SOLIDITY: f32 = 0.5;

/// A component channel of some document mode, for the §14.1 colorized
/// solo view (R/G/B, C/M/Y/K, or L/a/b per mode).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ComponentChannel {
    R,
    G,
    B,
    C,
    M,
    Y,
    K,
    /// Lab L* — colorized view is the neutral ramp (grayscale).
    L,
    /// Lab a* opponent axis (election: green → magenta ramp).
    AOpponent,
    /// Lab b* opponent axis (election: blue → yellow ramp).
    BOpponent,
}

/// Ink color (encoded working RGB) of a CMYK component at full coverage,
/// for the §14.1 "ink tint over white" view.
const fn ink_rgb(c: ComponentChannel) -> [f32; 3] {
    match c {
        ComponentChannel::C => [0.0, 1.0, 1.0],
        ComponentChannel::M => [1.0, 0.0, 1.0],
        ComponentChannel::Y => [1.0, 1.0, 0.0],
        ComponentChannel::K => [0.0, 0.0, 0.0],
        _ => [0.0, 0.0, 0.0],
    }
}

#[inline]
fn lerp3(a: [f32; 3], b: [f32; 3], w: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * w,
        a[1] + (b[1] - a[1]) * w,
        a[2] + (b[2] - a[2]) * w,
    ]
}

impl ComponentChannel {
    /// §14.1 [NORMATIVE] colorized value: `v` displays as `v · primary`
    /// for RGB components; CMYK components as the ink tint over white;
    /// Lab `a`/`b` as their opponent-axis ramps (pole election in the
    /// module docs); `L` as the neutral ramp. Output is working RGB for
    /// the §5 display transform.
    pub fn colorize(self, v: f32) -> [f32; 3] {
        match self {
            ComponentChannel::R => [v, 0.0, 0.0],
            ComponentChannel::G => [0.0, v, 0.0],
            ComponentChannel::B => [0.0, 0.0, v],
            // Ink tint over white: white at v = 0, the ink at v = 1.
            ComponentChannel::C
            | ComponentChannel::M
            | ComponentChannel::Y
            | ComponentChannel::K => lerp3([1.0, 1.0, 1.0], ink_rgb(self), v),
            ComponentChannel::L => [v, v, v],
            ComponentChannel::AOpponent => lerp3([0.0, 1.0, 0.0], [1.0, 0.0, 1.0], v),
            ComponentChannel::BOpponent => lerp3([0.0, 0.0, 1.0], [1.0, 1.0, 0.0], v),
        }
    }
}

/// §14.1 multi-solo: "Multi-solo sums the colorized components" (soloing
/// R+G shows the image minus blue). Summed per channel, clamped to the
/// working range top so ink sums cannot blow past white.
pub fn colorize_multi(picks: &[(ComponentChannel, f32)]) -> [f32; 3] {
    let mut acc = [0.0f32; 3];
    for &(c, v) in picks {
        let rgb = c.colorize(v);
        acc[0] += rgb[0];
        acc[1] += rgb[1];
        acc[2] += rgb[2];
    }
    [
        acc[0].clamp(0.0, 1.0),
        acc[1].clamp(0.0, 1.0),
        acc[2].clamp(0.0, 1.0),
    ]
}

/// The §13.4 / §14.1 rubylith overlay: tint where coverage is ABSENT
/// (`unselected area as a rubylith tint`), leaving covered pixels
/// untouched: `out = lerp(base, ink, (1 − coverage) · solidity)`.
/// Alpha-channel-over-composite (§14.1) passes the channel value as
/// `coverage` with the channel's `ink`/`solidity` display props; Quick
/// Mask's tint-selected-instead toggle is the caller inverting
/// `coverage`. Deterministic fixed-order scalar math.
pub fn rubylith_overlay(base: [f32; 3], coverage: f32, ink: [f32; 3], solidity: f32) -> [f32; 3] {
    lerp3(base, ink, (1.0 - coverage) * solidity)
}

/// §14.2 [NORMATIVE] persistence predicate: `.appname` stores an alpha
/// channel in the 8-bit encoding **only** when every value is exactly
/// representable there (`v = n/255` — binary and 8-bit-born masks), else
/// at f32 — "lossless-or-not-at-all". True iff every value round-trips
/// `f32 → n/255 → f32` bit-exactly with integral `n ∈ 0..=255`.
pub fn plane_is_u8_exact(values: &[f32]) -> bool {
    values.iter().all(|&v| {
        if !(0.0..=1.0).contains(&v) {
            return false;
        }
        let n = (v * 255.0).round();
        (n / 255.0) == v
    })
}

/// §14.3 spot-channel display props (reserved room, Post-1.0 semantics):
/// `(named ink, solidity)`. The DATA plane is coverage and never
/// converted; the *preview ink color* is what the §5 pipeline converts
/// for display (§6.2). `channel.add`'s `ink`/`solidity` params are
/// reserved against exactly this shape (§10.3).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpotDisplay {
    /// Preview ink, encoded working RGB (converted for display only).
    pub ink: [f32; 3],
    /// Solidity `0..=1`.
    pub solidity: f32,
}

impl Default for SpotDisplay {
    fn default() -> SpotDisplay {
        SpotDisplay {
            ink: RUBYLITH_DEFAULT_INK,
            solidity: RUBYLITH_DEFAULT_SOLIDITY,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §14.1 letter: R at v shows (v,0,0); full-coverage C ink over white
    /// is cyan; v=0 ink is white; L is neutral.
    #[test]
    fn colorize_matches_the_normative_table() {
        assert_eq!(ComponentChannel::R.colorize(0.25), [0.25, 0.0, 0.0]);
        assert_eq!(ComponentChannel::G.colorize(1.0), [0.0, 1.0, 0.0]);
        assert_eq!(ComponentChannel::B.colorize(0.5), [0.0, 0.0, 0.5]);
        assert_eq!(ComponentChannel::C.colorize(1.0), [0.0, 1.0, 1.0]);
        assert_eq!(ComponentChannel::C.colorize(0.0), [1.0, 1.0, 1.0]);
        assert_eq!(ComponentChannel::K.colorize(1.0), [0.0, 0.0, 0.0]);
        assert_eq!(ComponentChannel::L.colorize(0.7), [0.7, 0.7, 0.7]);
        // Opponent ramps hit their poles.
        assert_eq!(ComponentChannel::AOpponent.colorize(0.0), [0.0, 1.0, 0.0]);
        assert_eq!(ComponentChannel::AOpponent.colorize(1.0), [1.0, 0.0, 1.0]);
        assert_eq!(ComponentChannel::BOpponent.colorize(0.0), [0.0, 0.0, 1.0]);
        assert_eq!(ComponentChannel::BOpponent.colorize(1.0), [1.0, 1.0, 0.0]);
    }

    /// §14.1: soloing R+G shows the image minus blue — a white pixel
    /// (r=g=1) colorizes to yellow.
    #[test]
    fn multi_solo_sums_components() {
        let out = colorize_multi(&[(ComponentChannel::R, 1.0), (ComponentChannel::G, 1.0)]);
        assert_eq!(out, [1.0, 1.0, 0.0]);
        // Sum clamps at the top.
        let out = colorize_multi(&[
            (ComponentChannel::C, 0.2),
            (ComponentChannel::M, 0.2),
            (ComponentChannel::Y, 0.2),
        ]);
        assert!(out.iter().all(|&c| c <= 1.0));
    }

    /// §13.4: full coverage untouched; zero coverage tinted at solidity;
    /// defaults are red / 0.5.
    #[test]
    fn rubylith_tints_the_uncovered_area() {
        let base = [0.4f32, 0.4, 0.4];
        let covered = rubylith_overlay(base, 1.0, RUBYLITH_DEFAULT_INK, RUBYLITH_DEFAULT_SOLIDITY);
        assert_eq!(covered, base);
        let bare = rubylith_overlay(base, 0.0, RUBYLITH_DEFAULT_INK, RUBYLITH_DEFAULT_SOLIDITY);
        // lerp(base, red, 0.5) exactly.
        assert_eq!(bare, [0.4 + (1.0 - 0.4) * 0.5, 0.2, 0.2]);
        assert_eq!(RUBYLITH_DEFAULT_INK, [1.0, 0.0, 0.0]);
        assert_eq!(RUBYLITH_DEFAULT_SOLIDITY, 0.5);
    }

    /// §14.2 predicate: all 256 8-bit codes pass; values off the n/255
    /// lattice fail; out-of-range fails; the empty plane passes.
    #[test]
    fn u8_exactness_predicate() {
        let codes: Vec<f32> = (0u32..=255).map(|n| n as f32 / 255.0).collect();
        assert!(plane_is_u8_exact(&codes));
        assert!(plane_is_u8_exact(&[]));
        assert!(!plane_is_u8_exact(&[0.5])); // 0.5 is not n/255
        assert!(!plane_is_u8_exact(&[127.5 / 255.0]));
        assert!(!plane_is_u8_exact(&[-0.1]));
        assert!(!plane_is_u8_exact(&[1.1]));
        assert!(!plane_is_u8_exact(&[f32::NAN]));
        // A feathered (float-born) mask value fails, forcing f32 storage.
        // (Chosen off the n/255 lattice: 0.7·255 = 178.5 — note 1/3 IS
        // on it, 85/255 exactly, so "looks fractional" proves nothing.)
        assert!(!plane_is_u8_exact(&[0.7]));
    }

    /// Spot display defaults are the §14.1 rubylith defaults.
    #[test]
    fn spot_display_defaults() {
        let d = SpotDisplay::default();
        assert_eq!(d.ink, RUBYLITH_DEFAULT_INK);
        assert_eq!(d.solidity, RUBYLITH_DEFAULT_SOLIDITY);
    }
}
