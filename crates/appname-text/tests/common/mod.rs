// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Procedural spike fixture face ("AppnameSpike"): a minimal TrueType
//! font BUILT AS BYTES by this module — no font file is vendored
//! anywhere in the repo (§3.10 font-licensing rule; deps records cover
//! libraries only, and the bundled-Noto set is a CP-1.0 record). The
//! glyphs are axis-aligned rectangles with distinct, hand-picked
//! advances, so shaping and rasterization have EXACT oracles: HarfBuzz
//! must echo the hmtx advances and cmap ids verbatim (no GSUB/GPOS in
//! the face), and FreeType must fill the rectangles' exact pixel areas
//! at integer-aligned sizes.
//!
//! Layout (unitsPerEm = 1000, ascender 800, descender −200, lineGap 90):
//!
//! | gid | char    | advance | outline                     |
//! |-----|---------|---------|-----------------------------|
//! | 0   | .notdef | 500     | (empty)                     |
//! | 1   | U+0020  | 500     | (empty)                     |
//! | 2   | !       | 300     | rect x 100–200, y 0–700     |
//! | 3–8 | A–F     | 600+20k | rect x 100–(500+20k), y 0–700 |
//! | 9   | א U+05D0| 550     | rect x 80–470, y 0–600      |
//! | 10  | ב U+05D1| 560     | rect x 80–480, y 0–600      |
//! | 11  | 永 U+6C38| 1000   | rect x 50–950, y 0–760      |
//!
//! Tables: cmap(4) glyf head hhea hmtx loca(long) maxp(1.0) name(empty)
//! post(3.0). No OS/2 (metrics ride the §23.2 hhea arm), no GSUB/GPOS/
//! GDEF (shaping is pure cmap+hmtx — that exactness is the point), no
//! hinting bytecode (§23.2 forbids native bytecode anyway).

// Shared across several integration-test binaries; each binary uses its
// own slice of the helpers, so unqualified builds see the rest as dead
// (reviewer-suggested allow, C-3 re-review round).
#![allow(dead_code)]

pub const UPM: i32 = 1000;

/// (codepoint, advance, rect (x0, x1, y0, y1)); rect None = empty.
type GlyphRow = (char, u16, Option<(i16, i16, i16, i16)>);

const GLYPHS: [GlyphRow; 12] = [
    (' ', 500, None),
    ('!', 300, Some((100, 200, 0, 700))),
    ('A', 600, Some((100, 500, 0, 700))),
    ('B', 620, Some((100, 520, 0, 700))),
    ('C', 640, Some((100, 540, 0, 700))),
    ('D', 660, Some((100, 560, 0, 700))),
    ('E', 680, Some((100, 580, 0, 700))),
    ('F', 700, Some((100, 600, 0, 700))),
    // Descender glyph (C-5 re-review T-1: below-baseline ink so the
    // faux-italic left-shear path is testable; codepoint order kept
    // for the cmap segment builder).
    ('g', 620, Some((100, 520, -200, 500))),
    ('\u{05D0}', 550, Some((80, 470, 0, 600))),
    ('\u{05D1}', 560, Some((80, 480, 0, 600))),
    ('\u{6C38}', 1000, Some((50, 950, 0, 760))),
];

/// The known cmap: glyph id for a mapped char (gid = table row + 1).
pub fn gid(c: char) -> u32 {
    GLYPHS
        .iter()
        .position(|&(g, _, _)| g == c)
        .map(|i| i as u32 + 1)
        .expect("char not in the spike face")
}

/// The known hmtx advance, in font units.
pub fn advance(c: char) -> i32 {
    i32::from(GLYPHS[gid(c) as usize - 1].1)
}

/// The known outline rect (x0, x1, y0, y1) in font units.
pub fn rect(c: char) -> (i16, i16, i16, i16) {
    GLYPHS[gid(c) as usize - 1]
        .2
        .expect("char has no outline in the spike face")
}

fn be16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn bei16(out: &mut Vec<u8>, v: i16) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn be32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn pad4(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
}

fn checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    let mut chunk = [0u8; 4];
    for i in (0..data.len()).step_by(4) {
        chunk.fill(0);
        let n = (data.len() - i).min(4);
        chunk[..n].copy_from_slice(&data[i..i + n]);
        sum = sum.wrapping_add(u32::from_be_bytes(chunk));
    }
    sum
}

/// One rectangle glyph's glyf entry.
fn rect_glyph(x0: i16, x1: i16, y0: i16, y1: i16) -> Vec<u8> {
    let mut g = Vec::new();
    bei16(&mut g, 1); // numberOfContours
    bei16(&mut g, x0);
    bei16(&mut g, y0);
    bei16(&mut g, x1);
    bei16(&mut g, y1);
    be16(&mut g, 3); // endPtsOfContours[0]
    be16(&mut g, 0); // instructionLength
                     // 4 points, all on-curve (flag 0x01), 16-bit deltas.
    g.extend_from_slice(&[0x01; 4]);
    // Points: (x0,y0) (x1,y0) (x1,y1) (x0,y1) — x deltas then y deltas.
    for d in [x0, x1 - x0, 0, x0 - x1] {
        bei16(&mut g, d);
    }
    for d in [y0, 0, y1 - y0, 0] {
        bei16(&mut g, d);
    }
    pad4(&mut g);
    g
}

/// Spike sTypo metrics used when the OS/2 variant is built (distinct
/// from the hhea 800/−200/90 so the §23.2 metrics rule's two arms are
/// distinguishable in tests).
pub const TYPO: (i16, i16, i16) = (760, -240, 100);

/// Build the complete font (no OS/2: metrics ride the hhea arm).
/// Deterministic: same bytes every call.
pub fn build() -> Vec<u8> {
    build_inner(false, None)
}

/// Build the OS/2 variant: fsSelection bit 7 (USE_TYPO_METRICS) set,
/// sTypo* = [`TYPO`], decor/position fields ZERO (exercises the
/// zero-as-absent fallback arm).
pub fn build_with_typo_os2() -> Vec<u8> {
    build_inner(true, None)
}

/// §23.4 decor/position OS/2 values for the DECOR variant (nonzero, so
/// the FONT arm — including the subscript sign negation — is
/// exercisable; C-4 review MINOR).
pub const DECOR_STRIKEOUT: (i16, i16) = (300, 40); // (yStrikeoutPosition, ySize)
pub const DECOR_SUPER: (i16, i16) = (600, 250); // (ySuperscriptYSize, yOffset — positive-UP)
pub const DECOR_SUB: (i16, i16) = (620, 180); // (ySubscriptYSize, yOffset — positive-DOWN)

/// Build the OS/2 DECOR variant: same as [`build_with_typo_os2`] but
/// with nonzero strikeout/super/subscript metrics.
#[allow(dead_code)]
pub fn build_with_decor_os2() -> Vec<u8> {
    build_inner_decor(true, None, true)
}

/// Build a COVERAGE SUBSET of the spike face (§44.2 fallback tests
/// need faces with different cmaps). Glyph ids renumber; probe by
/// codepoint, not by [`gid`] (which describes the full face only).
#[allow(dead_code)] // each integration test binary uses its own slice of this module
pub fn build_subset(keep: &[char]) -> Vec<u8> {
    build_inner(false, Some(keep))
}

fn build_inner(with_os2: bool, keep: Option<&[char]>) -> Vec<u8> {
    build_inner_full(with_os2, keep, false, false)
}

fn build_inner_decor(with_os2: bool, keep: Option<&[char]>, decor: bool) -> Vec<u8> {
    build_inner_full(with_os2, keep, decor, false)
}

/// Build the LIGA variant: full face + a GSUB 'liga' lookup mapping
/// A+B → the 永 glyph (a reused target — no new outline needed). This
/// is the FEATURE-BEARING face the C-4/C-5 reviews called for: the
/// engine-default liga path and the run-feature later-wins overlay are
/// BEHAVIORALLY testable against it.
pub fn build_with_liga() -> Vec<u8> {
    build_inner_full(false, None, false, true)
}

/// Build the KERN variant: full face + a GPOS 'kern' pair adjusting
/// A+B by −80 units — the C-5 slice-2 review's D1 probe face: kerning
/// sets UNSAFE_TO_BREAK, and kerned LATIN must keep its tracking (the
/// Joining_Type gate rejects the suppression).
pub fn build_with_kern() -> Vec<u8> {
    build_inner_all(false, None, false, false, true)
}

/// The kern adjustment (font units, applied to A before B).
pub const KERN_AB: i16 = -80;

/// Hand-assembled GPOS: same list skeleton as the GSUB, feature
/// 'kern' → lookup 0 = PairPosFormat1 (XAdvance only): coverage
/// [gid(A)], PairSet A: {second: gid(B), value1.xAdvance = KERN_AB}.
/// Fixed 80-byte layout, offsets by construction.
fn gpos_kern() -> Vec<u8> {
    let (gid_a, gid_b) = (gid('A') as u16, gid('B') as u16);
    let mut t = Vec::new();
    be32(&mut t, 0x0001_0000);
    be16(&mut t, 10); // scriptList
    be16(&mut t, 30); // featureList
    be16(&mut t, 44); // lookupList
                      // ScriptList @10
    be16(&mut t, 1);
    t.extend_from_slice(b"DFLT");
    be16(&mut t, 8);
    // Script @18
    be16(&mut t, 4);
    be16(&mut t, 0);
    // LangSys @22
    be16(&mut t, 0);
    be16(&mut t, 0xFFFF);
    be16(&mut t, 1);
    be16(&mut t, 0);
    // FeatureList @30
    be16(&mut t, 1);
    t.extend_from_slice(b"kern");
    be16(&mut t, 8);
    // Feature @38
    be16(&mut t, 0);
    be16(&mut t, 1);
    be16(&mut t, 0);
    // LookupList @44
    be16(&mut t, 1);
    be16(&mut t, 4);
    // Lookup @48
    be16(&mut t, 2); // type: PairPos
    be16(&mut t, 0);
    be16(&mut t, 1);
    be16(&mut t, 8);
    // PairPosFormat1 @56
    be16(&mut t, 1); // format
    be16(&mut t, 12); // coverage @ sub+12
    be16(&mut t, 0x0004); // valueFormat1: XAdvance
    be16(&mut t, 0); // valueFormat2
    be16(&mut t, 1); // pairSetCount
    be16(&mut t, 18); // PairSet @ sub+18
                      // Coverage @68
    be16(&mut t, 1);
    be16(&mut t, 1);
    be16(&mut t, gid_a);
    // PairSet @74
    be16(&mut t, 1);
    be16(&mut t, gid_b);
    bei16(&mut t, KERN_AB);
    debug_assert_eq!(t.len(), 80);
    t
}

/// Hand-assembled GSUB: ScriptList(DFLT→dflt LangSys→feature 0),
/// FeatureList('liga'→lookup 0), LookupList(type-4 LigatureSubst:
/// coverage [gid(A)], A: Ligature{lig=gid(永), comps=[gid(B)]}).
/// Offsets verified by construction (fixed layout, 80 bytes).
fn gsub_liga() -> Vec<u8> {
    let (gid_a, gid_b, gid_lig) = (gid('A') as u16, gid('B') as u16, gid('\u{6C38}') as u16);
    let mut t = Vec::new();
    be32(&mut t, 0x0001_0000); // version 1.0
    be16(&mut t, 10); // scriptListOffset
    be16(&mut t, 30); // featureListOffset
    be16(&mut t, 44); // lookupListOffset
                      // ScriptList @10
    be16(&mut t, 1);
    t.extend_from_slice(b"DFLT");
    be16(&mut t, 8); // Script @ SL+8
                     // Script @18
    be16(&mut t, 4); // defaultLangSys @ Script+4
    be16(&mut t, 0); // langSysCount
                     // LangSys @22
    be16(&mut t, 0); // lookupOrder
    be16(&mut t, 0xFFFF); // requiredFeatureIndex
    be16(&mut t, 1); // featureIndexCount
    be16(&mut t, 0);
    // FeatureList @30
    be16(&mut t, 1);
    t.extend_from_slice(b"liga");
    be16(&mut t, 8); // Feature @ FL+8
                     // Feature @38
    be16(&mut t, 0); // featureParams
    be16(&mut t, 1); // lookupIndexCount
    be16(&mut t, 0);
    // LookupList @44
    be16(&mut t, 1);
    be16(&mut t, 4); // Lookup @ LL+4
                     // Lookup @48
    be16(&mut t, 4); // type: LigatureSubst
    be16(&mut t, 0); // flag
    be16(&mut t, 1); // subTableCount
    be16(&mut t, 8); // subtable @ Lookup+8
                     // LigatureSubstFormat1 @56
    be16(&mut t, 1); // format
    be16(&mut t, 8); // coverage @ sub+8
    be16(&mut t, 1); // ligSetCount
    be16(&mut t, 14); // LigatureSet @ sub+14
                      // Coverage @64
    be16(&mut t, 1); // format 1
    be16(&mut t, 1); // glyphCount
    be16(&mut t, gid_a);
    // LigatureSet @70
    be16(&mut t, 1); // ligatureCount
    be16(&mut t, 4); // Ligature @ LigSet+4
                     // Ligature @74
    be16(&mut t, gid_lig);
    be16(&mut t, 2); // compCount (incl. the coverage glyph)
    be16(&mut t, gid_b);
    debug_assert_eq!(t.len(), 80);
    t
}

fn build_inner_full(with_os2: bool, keep: Option<&[char]>, decor: bool, liga: bool) -> Vec<u8> {
    build_inner_all(with_os2, keep, decor, liga, false)
}

fn build_inner_all(
    with_os2: bool,
    keep: Option<&[char]>,
    decor: bool,
    liga: bool,
    kern: bool,
) -> Vec<u8> {
    let glyph_rows: Vec<GlyphRow> = GLYPHS
        .iter()
        .filter(|(c, _, _)| keep.is_none_or(|k| k.contains(c)))
        .copied()
        .collect();
    let glyphs = glyph_rows.as_slice(); // the body reads the
                                        // (possibly) filtered set
    let num_glyphs = glyphs.len() as u16 + 1; // + .notdef

    // ---- glyf + loca -------------------------------------------------
    let mut glyf = Vec::new();
    let mut loca: Vec<u32> = vec![0]; // .notdef: empty (offset 0..0)
    loca.push(glyf.len() as u32);
    for &(_, _, r) in glyphs {
        if let Some((x0, x1, y0, y1)) = r {
            glyf.extend_from_slice(&rect_glyph(x0, x1, y0, y1));
        }
        loca.push(glyf.len() as u32);
    }
    debug_assert_eq!(loca.len(), num_glyphs as usize + 1);
    let mut loca_bytes = Vec::new();
    for off in &loca {
        be32(&mut loca_bytes, *off);
    }

    // Global bounds over the rect glyphs.
    let (mut xmin, mut ymin, mut xmax, mut ymax) = (i16::MAX, i16::MAX, i16::MIN, i16::MIN);
    for &(_, _, r) in glyphs {
        if let Some((x0, x1, y0, y1)) = r {
            xmin = xmin.min(x0);
            xmax = xmax.max(x1);
            ymin = ymin.min(y0);
            ymax = ymax.max(y1);
        }
    }

    // ---- head --------------------------------------------------------
    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000); // version 1.0
    be32(&mut head, 0x0001_0000); // fontRevision
    be32(&mut head, 0); // checkSumAdjustment (patched last)
    be32(&mut head, 0x5F0F_3CF5); // magicNumber
    be16(&mut head, 0); // flags
    be16(&mut head, UPM as u16); // unitsPerEm
    be32(&mut head, 0); // created hi (epoch zero: deterministic)
    be32(&mut head, 0); // created lo
    be32(&mut head, 0); // modified hi
    be32(&mut head, 0); // modified lo
    bei16(&mut head, xmin);
    bei16(&mut head, ymin);
    bei16(&mut head, xmax);
    bei16(&mut head, ymax);
    be16(&mut head, 0); // macStyle
    be16(&mut head, 6); // lowestRecPPEM
    bei16(&mut head, 2); // fontDirectionHint
    bei16(&mut head, 1); // indexToLocFormat: long
    bei16(&mut head, 0); // glyphDataFormat

    // ---- hhea --------------------------------------------------------
    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    bei16(&mut hhea, 800); // ascender
    bei16(&mut hhea, -200); // descender
    bei16(&mut hhea, 90); // lineGap
    be16(&mut hhea, 1000); // advanceWidthMax
    bei16(&mut hhea, 50); // minLeftSideBearing
    bei16(&mut hhea, 50); // minRightSideBearing
    bei16(&mut hhea, xmax); // xMaxExtent
    bei16(&mut hhea, 1); // caretSlopeRise
    bei16(&mut hhea, 0); // caretSlopeRun
    bei16(&mut hhea, 0); // caretOffset
    for _ in 0..4 {
        bei16(&mut hhea, 0); // reserved
    }
    bei16(&mut hhea, 0); // metricDataFormat
    be16(&mut hhea, num_glyphs); // numberOfHMetrics (full metrics)

    // ---- maxp --------------------------------------------------------
    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0001_0000);
    be16(&mut maxp, num_glyphs);
    be16(&mut maxp, 4); // maxPoints
    be16(&mut maxp, 1); // maxContours
    be16(&mut maxp, 0); // maxCompositePoints
    be16(&mut maxp, 0); // maxCompositeContours
    be16(&mut maxp, 2); // maxZones
    be16(&mut maxp, 0); // maxTwilightPoints
    be16(&mut maxp, 0); // maxStorage
    be16(&mut maxp, 0); // maxFunctionDefs
    be16(&mut maxp, 0); // maxInstructionDefs
    be16(&mut maxp, 0); // maxStackElements
    be16(&mut maxp, 0); // maxSizeOfInstructions
    be16(&mut maxp, 0); // maxComponentElements
    be16(&mut maxp, 0); // maxComponentDepth

    // ---- hmtx --------------------------------------------------------
    let mut hmtx = Vec::new();
    be16(&mut hmtx, 500); // .notdef advance
    bei16(&mut hmtx, 0); // .notdef lsb
    for &(_, adv, r) in glyphs {
        be16(&mut hmtx, adv);
        bei16(&mut hmtx, r.map_or(0, |(x0, _, _, _)| x0));
    }

    // ---- cmap (format 4, platform 3 encoding 1) ----------------------
    // Segments over contiguous (codepoint, gid) runs; gids are assigned
    // in codepoint order above, so idDelta arithmetic suffices.
    let mut segments: Vec<(u16, u16, i32)> = Vec::new(); // (start, end, gid_of_start)
    {
        let mut i = 0;
        while i < glyphs.len() {
            let start_cp = glyphs[i].0 as u32;
            let start_gid = i as u32 + 1;
            let mut j = i;
            while j + 1 < glyphs.len() && glyphs[j + 1].0 as u32 == glyphs[j].0 as u32 + 1 {
                j += 1;
            }
            let end_cp = glyphs[j].0 as u32;
            assert!(end_cp <= 0xFFFF, "spike face is BMP-only (format 4)");
            segments.push((start_cp as u16, end_cp as u16, start_gid as i32));
            i = j + 1;
        }
    }
    segments.push((0xFFFF, 0xFFFF, -1)); // terminator: idDelta 1 → gid 0
    let seg_count = segments.len() as u16;
    let seg_count_x2 = seg_count * 2;
    let search_range = 2 * (1u16 << (15 - seg_count.leading_zeros() as u16));
    let entry_selector = (15 - seg_count.leading_zeros()) as u16;
    let range_shift = seg_count_x2 - search_range;

    let mut sub4 = Vec::new();
    be16(&mut sub4, 4); // format
    be16(&mut sub4, 16 + 8 * seg_count); // length
    be16(&mut sub4, 0); // language
    be16(&mut sub4, seg_count_x2);
    be16(&mut sub4, search_range);
    be16(&mut sub4, entry_selector);
    be16(&mut sub4, range_shift);
    for &(_, end, _) in &segments {
        be16(&mut sub4, end);
    }
    be16(&mut sub4, 0); // reservedPad
    for &(start, _, _) in &segments {
        be16(&mut sub4, start);
    }
    for &(start, _, gid0) in &segments {
        let delta: i32 = if gid0 < 0 { 1 } else { gid0 - i32::from(start) };
        bei16(&mut sub4, (delta & 0xFFFF) as u16 as i16); // idDelta mod 2^16
    }
    for _ in &segments {
        be16(&mut sub4, 0); // idRangeOffset
    }

    let mut cmap = Vec::new();
    be16(&mut cmap, 0); // version
    be16(&mut cmap, 1); // numTables
    be16(&mut cmap, 3); // platformID (Windows)
    be16(&mut cmap, 1); // encodingID (Unicode BMP)
    be32(&mut cmap, 12); // subtable offset
    cmap.extend_from_slice(&sub4);

    // ---- name (present but empty) + post (3.0) -----------------------
    let mut name = Vec::new();
    be16(&mut name, 0); // format
    be16(&mut name, 0); // count
    be16(&mut name, 6); // stringOffset
    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000); // version 3.0: no glyph names
    be32(&mut post, 0); // italicAngle
    bei16(&mut post, -75); // underlinePosition
    bei16(&mut post, 50); // underlineThickness
    be32(&mut post, 0); // isFixedPitch
    for _ in 0..4 {
        be32(&mut post, 0); // min/max Mem*
    }

    // ---- OS/2 v4 (variant only) --------------------------------------
    // Minimal but complete 96-byte version-4 table; only the fields the
    // §23.2 metrics rule reads carry meaning (fsSelection bit 7 +
    // sTypo*); usWin* mirror hhea so nothing else keys off them.
    let mut os2 = Vec::new();
    if with_os2 {
        be16(&mut os2, 4); // version
        bei16(&mut os2, 500); // xAvgCharWidth
        be16(&mut os2, 400); // usWeightClass
        be16(&mut os2, 5); // usWidthClass
        be16(&mut os2, 0); // fsType (installable embedding)
        let (sk_pos, sk_size) = if decor { DECOR_STRIKEOUT } else { (0, 0) };
        let (sup_size, sup_off) = if decor { DECOR_SUPER } else { (0, 0) };
        let (sub_size, sub_off) = if decor { DECOR_SUB } else { (0, 0) };
        bei16(&mut os2, 0); // ySubscriptXSize
        bei16(&mut os2, sub_size); // ySubscriptYSize
        bei16(&mut os2, 0); // ySubscriptXOffset
        bei16(&mut os2, sub_off); // ySubscriptYOffset (positive-DOWN)
        bei16(&mut os2, 0); // ySuperscriptXSize
        bei16(&mut os2, sup_size); // ySuperscriptYSize
        bei16(&mut os2, 0); // ySuperscriptXOffset
        bei16(&mut os2, sup_off); // ySuperscriptYOffset (positive-UP)
        bei16(&mut os2, sk_size); // yStrikeoutSize
        bei16(&mut os2, sk_pos); // yStrikeoutPosition
        bei16(&mut os2, 0); // sFamilyClass
        os2.extend_from_slice(&[0u8; 10]); // panose
        for _ in 0..4 {
            be32(&mut os2, 0); // ulUnicodeRange1..4
        }
        os2.extend_from_slice(b"APNM"); // achVendID
        be16(&mut os2, 0x0080); // fsSelection: bit 7 USE_TYPO_METRICS
        be16(&mut os2, 0x0020); // usFirstCharIndex (space)
        be16(&mut os2, 0x6C38); // usLastCharIndex
        bei16(&mut os2, TYPO.0); // sTypoAscender
        bei16(&mut os2, TYPO.1); // sTypoDescender
        bei16(&mut os2, TYPO.2); // sTypoLineGap
        be16(&mut os2, 800); // usWinAscent
        be16(&mut os2, 200); // usWinDescent
        be32(&mut os2, 0); // ulCodePageRange1
        be32(&mut os2, 0); // ulCodePageRange2
        bei16(&mut os2, 0); // sxHeight
        bei16(&mut os2, 700); // sCapHeight
        be16(&mut os2, 0); // usDefaultChar
        be16(&mut os2, 0x0020); // usBreakChar
        be16(&mut os2, 1); // usMaxContext
    }

    // ---- assemble (directory sorted by tag) --------------------------
    let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"cmap", cmap),
        (*b"glyf", glyf),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca_bytes),
        (*b"maxp", maxp),
        (*b"name", name),
        (*b"post", post),
    ];
    if with_os2 {
        // "OS/2" sorts between "OS" prefixes and lowercase tags: byte
        // order puts it FIRST among these tags (0x4F < all lowercase).
        tables.insert(0, (*b"OS/2", os2));
    }
    if liga {
        // "GSUB" (0x47…) sorts before "OS/2" and every lowercase tag.
        tables.insert(0, (*b"GSUB", gsub_liga()));
    }
    if kern {
        // "GPOS" sorts before "GSUB" (0x50 < 0x53).
        tables.insert(0, (*b"GPOS", gpos_kern()));
    }
    let num_tables = tables.len() as u16;
    let sr = 16 * (1u16 << (15 - num_tables.leading_zeros() as u16));
    let es = (15 - num_tables.leading_zeros()) as u16;

    let mut font = Vec::new();
    be32(&mut font, 0x0001_0000); // sfntVersion
    be16(&mut font, num_tables);
    be16(&mut font, sr);
    be16(&mut font, es);
    be16(&mut font, num_tables * 16 - sr);

    let mut offset = 12 + 16 * tables.len() as u32;
    let mut head_offset = 0u32;
    let mut records = Vec::new();
    for (tag, data) in &mut tables {
        pad4(data);
        if tag == b"head" {
            head_offset = offset;
        }
        records.push((*tag, checksum(data), offset, data.len() as u32));
        offset += data.len() as u32;
    }
    for &(tag, sum, off, len) in &records {
        font.extend_from_slice(&tag);
        be32(&mut font, sum);
        be32(&mut font, off);
        // Length: actual (unpadded) length is conventional; padded is
        // also legal — we recorded padded above for simplicity.
        be32(&mut font, len);
    }
    for (_, data) in &tables {
        font.extend_from_slice(data);
    }

    // checkSumAdjustment = 0xB1B0AFBA − checksum(entire font).
    let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&font));
    let at = head_offset as usize + 8;
    font[at..at + 4].copy_from_slice(&adjustment.to_be_bytes());
    font
}
