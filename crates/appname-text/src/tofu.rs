// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Procedural tofu (CP-1.0 slice C-3): the §44.2 step-6 / §23.5
//! rendering — "characters no chain covers render procedurally as hex
//! boxes (`.notdef` tofu showing the code point) — no font dependency,
//! nothing silently invisible."
//!
//! The box is drawn from an authored 3×5 hex-digit micro-font (bit
//! patterns below — original, trivial, no external source), scaled by
//! integer nearest-neighbor so output is deterministic and blocky at
//! every size. Output is the standard [`GlyphCoverage`] Gray f32 plane,
//! so tofu composites through exactly the pipeline every real glyph
//! uses (§23.2).
//!
//! Pinned metrics (spec gives none for tofu; recorded for the fold-in
//! register): design grid 15 units tall (border 1 + pad 1 + two 5-unit
//! digit rows + 1 gap + pad 1 + border 1), scaled by
//! `k = max(1, floor(0.7·em / 15))`; the box sits ON the baseline and
//! extends 15k up; side bearing k; advance = box width + 2k. Code
//! points render as 4–6 uppercase hex digits (`{:04X}`), row-major in
//! two rows of `ceil(n/2)` columns.

use crate::pipeline::{Item, ShapedGlyph};
use crate::raster::GlyphCoverage;

/// 3×5 hex digits, one `[u8; 5]` of 3-bit rows each (MSB = left).
const DIGITS: [[u8; 5]; 16] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b110, 0b010, 0b010, 0b111], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b111, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b111, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b001, 0b001, 0b001], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b111], // 9
    [0b010, 0b101, 0b111, 0b101, 0b101], // A
    [0b110, 0b101, 0b110, 0b101, 0b110], // B
    [0b111, 0b100, 0b100, 0b100, 0b111], // C
    [0b110, 0b101, 0b101, 0b101, 0b110], // D
    [0b111, 0b100, 0b111, 0b100, 0b111], // E
    [0b111, 0b100, 0b111, 0b100, 0b100], // F
];

const DESIGN_H: usize = 15;

fn scale_for(em_px: f64) -> usize {
    (((0.7 * em_px) / DESIGN_H as f64).floor() as usize).max(1)
}

fn hex_digits(cp: char) -> Vec<u8> {
    format!("{:04X}", cp as u32)
        .bytes()
        .map(|b| match b {
            b'0'..=b'9' => b - b'0',
            _ => b - b'A' + 10,
        })
        .collect()
}

/// Synthesize the "shaped" glyphs for a [`crate::fontdb::TOFU_FACE`]
/// item: HarfBuzz never sees tofu (there is no face), so the CONVENTION
/// — one glyph per character, `glyph` = the CODE POINT, advance =
/// [`tofu_advance`] — lives here as its single implementation; the
/// §23.2 cluster map stays byte-indexed like every real run, and the
/// compose binding renders via [`tofu_coverage`].
pub fn tofu_shape(item: &Item, text: &str, em_px: f64) -> Vec<ShapedGlyph> {
    text[item.range.clone()]
        .char_indices()
        .map(|(i, c)| ShapedGlyph {
            glyph: c as u32,
            cluster: item.range.start + i,
            advance: tofu_advance(c, em_px),
            offset_x: 0.0,
            offset_y: 0.0,
            unsafe_to_break: false,
        })
        .collect()
}

/// Advance width of the tofu box for a code point at an em size
/// (layout consumes this where a shaped advance would stand).
pub fn tofu_advance(cp: char, em_px: f64) -> f64 {
    let cols = hex_digits(cp).len().div_ceil(2);
    let w_design = 4 + 4 * cols - 1; // 2 border+pad each side + digits
    let k = scale_for(em_px);
    ((w_design + 2) * k) as f64
}

/// Render the hex box for one code point (§23.5). Deterministic.
pub fn tofu_coverage(cp: char, em_px: f64) -> GlyphCoverage {
    let digits = hex_digits(cp);
    let cols = digits.len().div_ceil(2);
    let w = 4 + 4 * cols - 1;
    let h = DESIGN_H;
    let mut grid = vec![false; w * h];
    // Border outline.
    for x in 0..w {
        grid[x] = true;
        grid[(h - 1) * w + x] = true;
    }
    for y in 0..h {
        grid[y * w] = true;
        grid[y * w + (w - 1)] = true;
    }
    // Digits, row-major: row 0 gets the first `cols`, row 1 the rest.
    for (i, &d) in digits.iter().enumerate() {
        let (row, col) = (i / cols, i % cols);
        let ox = 2 + col * 4;
        let oy = 2 + row * 6;
        for (dy, bits) in DIGITS[d as usize].iter().enumerate() {
            for dx in 0..3 {
                if bits & (0b100 >> dx) != 0 {
                    grid[(oy + dy) * w + ox + dx] = true;
                }
            }
        }
    }
    // Integer nearest-neighbor upscale.
    let k = scale_for(em_px);
    let (pw, ph) = (w * k, h * k);
    let mut data = vec![0.0f32; pw * ph];
    for y in 0..ph {
        for x in 0..pw {
            if grid[(y / k) * w + (x / k)] {
                data[y * pw + x] = 1.0;
            }
        }
    }
    GlyphCoverage {
        left: k as i32,
        top: ph as i32, // box sits on the baseline, extends up
        width: pw as u32,
        height: ph as u32,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tofu_is_deterministic_and_never_empty() {
        let a = tofu_coverage('\u{03B1}', 16.0);
        let b = tofu_coverage('\u{03B1}', 16.0);
        assert_eq!(a, b);
        assert!(a.total() > 0.0, "nothing silently invisible");
        // Border corners present.
        assert_eq!(a.data[0], 1.0);
        assert_eq!(a.data[(a.width - 1) as usize], 1.0);
    }

    #[test]
    fn distinct_code_points_render_distinct_boxes() {
        let a = tofu_coverage('\u{0041}', 32.0);
        let b = tofu_coverage('\u{0042}', 32.0);
        assert_ne!(a.data, b.data);
    }

    #[test]
    fn astral_code_points_widen_the_box() {
        // U+1F600: 5 hex digits → 3 columns; BMP U+0041: 4 → 2 columns.
        let bmp = tofu_coverage('\u{0041}', 32.0);
        let ast = tofu_coverage('\u{1F600}', 32.0);
        assert!(ast.width > bmp.width);
        assert!(tofu_advance('\u{1F600}', 32.0) > tofu_advance('\u{0041}', 32.0));
    }

    #[test]
    fn scale_tracks_em_and_metrics_are_pinned() {
        let small = tofu_coverage('A', 16.0); // k = 1 (0.7·16/15 < 1 → 1)
        let big = tofu_coverage('A', 64.0); // k = floor(44.8/15) = 2
        assert_eq!(small.height, 15);
        assert_eq!(big.height, 30);
        assert_eq!(small.top, 15, "box sits on the baseline");
        assert_eq!(
            tofu_advance('A', 16.0),
            f64::from(small.width) + 2.0,
            "advance = box + two side bearings at k=1"
        );
    }
}
