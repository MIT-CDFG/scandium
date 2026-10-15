// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §23.3 layout: point, paragraph, on-path (CP-1.0 text-engine row,
//! slice C-1 — the layout core over the shaping pipeline).
//!
//! Spec: ES §23.3 [NORMATIVE v1] greedy first-fit line breaking at
//! UAX #14 opportunities restricted to cluster boundaries; mandatory LF;
//! a single cluster wider than the measure overflows alone; no
//! hyphenation in v1 (the field is parsed elsewhere and inert).
//! Justification distribution 1→2→3 exactly as written (word spaces
//! +200 % cap, letter-spacing ±2 % em, single-word +5 % em then
//! start-aligned). Leading = `max(autoLeadingPct × size, ascent +
//! descent + lineGap)` per line over its runs; fixed leading overrides;
//! first baseline at `box.top + first-line ascent`. Paragraph spacing/
//! indents per §23.3 (adjacent spacing adds; negative first-line indent
//! = hanging, clamped into the box; the scheme mirrors under RTL).
//! Text-on-path walks the §20.1 arc-length table (DD ㉕: the ONE path
//! model — [`appname_vector::flatten::ArcLengthTable`]) with the §23.3
//! s_mid/normal-offset/tangent rules and the tight-curvature stretch
//! band [1, 1.25].
//!
//! Pinned interpretations (recorded in the lane log for the fold-in
//! register; each is deterministic and tested):
//! * Trailing White_Space clusters immediately before a break
//!   opportunity do not count against the measure (hanging spaces —
//!   §23.3 is silent; the standard typographic reading).
//! * The stretch ramp inside the normative [1, 1.25] band is
//!   `min(1.25, em / radius)`, applied when the curvature radius at
//!   `s_mid` is below the cluster's em AND the glyph side is the bend's
//!   outer side (baseline-on-path counts as outer).
//! * A justified line with no expandable spaces letter-spaces up to
//!   +5 % em; if the line still cannot fill, it renders start-aligned
//!   with no synthetic spread (the "falls back to start-aligned"
//!   reading; the alternate render-at-cap reading is flagged to the
//!   fold-in register per the C-1 review).
//! * Point-text anchor alignment is literal left/center/right (§23.3
//!   "Anchors" wording), not base-direction-relative.
//! * A mandatory-break-terminated line counts as a paragraph-last line
//!   for `justifyLastLine` (tested). Note the paragraph contract:
//!   callers split REAL paragraphs on LF before calling
//!   [`layout_paragraph`] (per-paragraph indents/spacing re-trigger at
//!   that level); an interior LF here breaks the line and ends
//!   justification, nothing more.
//! * When NO UAX #14 opportunity fits on a line, the emergency break
//!   extends to the last FITTING cluster (never one cluster at a
//!   time; a long word fills the measure before overflowing to the
//!   next line — only a single cluster wider than the measure
//!   overflows alone, which is §23.3's own rule).
//!
//! What this slice does NOT do (C-2+): §23.4 style fields beyond size/
//! leading (tracking, kerning, caps, sub/super, underline), §44.2
//! FontDb/fallback chain, hyphenation (reserved), vertical text,
//! rasterization binding to tiles.

use core::ops::Range;

use appname_vector::flatten::ArcLengthTable;
use appname_vector::geom::Vec2;

use crate::pipeline::{visual_order, FaceId, Item, ShapedGlyph};
use crate::raster::FaceMetricsPx;

/// One shaped item plus the per-item style facts layout consumes
/// (the lean C-1 style surface; the full §23.4 model is C-2).
#[derive(Clone, Debug)]
pub struct StyledItem {
    pub item: Item,
    /// Shaper output, buffer order (visual within the item).
    pub glyphs: Vec<ShapedGlyph>,
    pub em_px: f64,
    pub metrics: FaceMetricsPx,
    /// §23.4 tracking in px (‰ of em resolved by the caller; 0 =
    /// none). Realized as the n−1 gaps between visually adjacent
    /// NON-ABSORBED body clusters with bidi-level-aware ownership (see
    /// place_line's plan) — no gap past either line edge, width == ink
    /// for any style/direction mix, no gap inside ligatures, and
    /// tracking suppressed at UNSAFE_TO_BREAK join boundaries (§23.4,
    /// C-5 slice 2).
    pub tracking_px: f64,
}

/// §23.3 leading model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Leading {
    Auto,
    /// Exact baseline-to-baseline step, px.
    Fixed(f64),
}

/// §23.4 alignment (start/end-relative under the paragraph base
/// direction, §23.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    Justify,
}

/// §23.3 last-line option for justified paragraphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JustifyLast {
    Start,
    Center,
    End,
    Full,
}

/// Lean §23.4 paragraph style (defaults per the panel table).
#[derive(Clone, Debug)]
pub struct ParagraphStyle {
    pub align: Align,
    pub justify_last: JustifyLast,
    /// `autoLeadingPct` (fraction; §23.4 default 120 %).
    pub auto_leading_pct: f64,
    pub leading: Leading,
    pub indent_left: f64,
    pub indent_right: f64,
    /// May be negative (hanging), clamped into the box (§23.3).
    pub indent_first: f64,
    pub space_before: f64,
    pub space_after: f64,
    /// Resolved base direction (the §23.2 auto/first-strong resolution
    /// and the §23.4 override both happen before layout).
    pub rtl_base: bool,
}

impl Default for ParagraphStyle {
    fn default() -> Self {
        ParagraphStyle {
            align: Align::Start,
            justify_last: JustifyLast::Start,
            auto_leading_pct: 1.2,
            leading: Leading::Auto,
            indent_left: 0.0,
            indent_right: 0.0,
            indent_first: 0.0,
            space_before: 0.0,
            space_after: 0.0,
            rtl_base: false,
        }
    }
}

/// Paragraph/area text box (doc px; plain f64 edges).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutBox {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// One measured cluster (the §23.2 cluster map's layout-side face).
#[derive(Clone, Debug)]
struct Cluster {
    range: Range<usize>,
    width: f64,
    is_space: bool,
    mandatory_after: bool,
    em_px: f64,
    item_index: usize,
    tracking_px: f64,
    /// No glyph was attributed to this grapheme cluster: it sits
    /// inside a shaping cluster (a formed ligature spans it) OR the
    /// shaper DELETED its content (HarfBuzz drops default-ignorables
    /// on faces without a space glyph — review D2). PINNED either way:
    /// gaps never form at absorbed boundaries — tracking inside a
    /// ligature would split the glyph, and a deleted-invisible cluster
    /// exposes no boundary (register: the zero-width-cluster gap
    /// policy differs across faces only in that a face WITH an
    /// invisible glyph yields a real cluster and a real gap; fold-in).
    absorbed: bool,
    /// The boundary BEFORE this cluster is a CURSIVE JOIN: a glyph
    /// starting it carries UNSAFE_TO_BREAK **and** the adjacent
    /// characters actually join per Unicode Joining_Type (the review-D1
    /// gate — UNSAFE_TO_BREAK alone is a superset set by kerning/marks/
    /// contextual lookups, and kerned Latin must keep its tracking).
    /// Tracking is suppressed at such boundaries (§23.4); justification
    /// letter-spacing keeps its own §23.3 authority — fold-in note.
    join_before: bool,
}

/// One positioned glyph: pen-relative baseline position in doc px.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedGlyph {
    pub glyph: u32,
    pub face: FaceId,
    /// Paragraph byte index (cluster map key).
    pub cluster: usize,
    pub x: f64,
    pub y: f64,
    /// The glyph's advance (px) — decoration spans and hit metrics
    /// consume it (C-5 review T-2: rects span ADVANCES, not em).
    pub advance: f64,
}

/// One laid-out line.
#[derive(Clone, Debug, PartialEq)]
pub struct LaidLine {
    /// Logical byte range of the paragraph text on this line.
    pub range: Range<usize>,
    pub baseline: f64,
    /// Natural (unjustified) advance width, hanging spaces excluded.
    pub width: f64,
    pub glyphs: Vec<PlacedGlyph>,
}

/// Paragraph layout result.
#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphLayout {
    pub lines: Vec<LaidLine>,
    /// §23.3 overflow: text past the last fully fitting line is
    /// preserved, not rendered; `first_hidden` is its byte start.
    pub overflowed: bool,
    pub first_hidden: Option<usize>,
    /// y just below the last rendered line's descent (spaceAfter and
    /// the next paragraph stack from here).
    pub bottom: f64,
}

// ---------------------------------------------------------------------
// Measurement
// ---------------------------------------------------------------------

/// Build measured clusters from grapheme bounds + shaped items.
/// Cluster width = Σ advances of its glyphs (§23.2 cluster map);
/// White_Space classification is the base char's `White_Space` property
/// (Rust's `char::is_whitespace` IS that property); LF/CR clusters are
/// mandatory-after and render nothing.
fn measure_clusters(text: &str, items: &[StyledItem], bounds: &[usize]) -> Vec<Cluster> {
    debug_assert!(bounds.first() == Some(&0) && bounds.last() == Some(&text.len()));
    let mut clusters: Vec<Cluster> = bounds
        .windows(2)
        .map(|w| {
            let range = w[0]..w[1];
            let ctext = &text[range.clone()];
            let base = ctext.chars().next().unwrap_or('\u{0}');
            Cluster {
                range,
                width: 0.0,
                is_space: base.is_whitespace(),
                mandatory_after: matches!(base, '\n' | '\r' | '\u{2028}' | '\u{2029}'),
                em_px: 0.0,
                item_index: usize::MAX,
                tracking_px: 0.0,
                absorbed: false,
                join_before: false,
            }
        })
        .collect();
    // Attribute glyph advances (and the owning item) per cluster.
    for (ii, si) in items.iter().enumerate() {
        for g in &si.glyphs {
            let ci = match bounds.binary_search(&g.cluster) {
                Ok(i) => i,
                Err(i) => i - 1,
            };
            let ci = ci.min(clusters.len() - 1);
            let c_start = clusters[ci].range.start;
            let c = &mut clusters[ci];
            if !c.mandatory_after {
                c.width += g.advance;
            }
            c.em_px = si.em_px;
            c.item_index = ii;
            c.tracking_px = si.tracking_px;
            if g.unsafe_to_break
                && crate::providers::is_cursive_join_boundary(&text[..c_start], &text[c_start..])
            {
                c.join_before = true;
            }
        }
    }
    // Clusters no glyph claimed sit INSIDE a shaping cluster (formed
    // ligatures span multiple graphemes at cluster level 1): mark them
    // ABSORBED — no gap ever forms at their boundaries — and inherit
    // the surrounding item so pens keep walking.
    let mut last = 0usize;
    for c in clusters.iter_mut() {
        if c.item_index == usize::MAX {
            c.item_index = last;
            if !c.mandatory_after {
                c.absorbed = true;
            }
        } else {
            last = c.item_index;
        }
        if c.em_px == 0.0 {
            c.em_px = items.get(c.item_index).map_or(0.0, |s| s.em_px);
        }
    }
    clusters
}

/// Width of `clusters[a..b]` excluding trailing White_Space (the
/// hanging-space measure; pinned interpretation, module header).
fn measure_width(clusters: &[Cluster], a: usize, b: usize) -> f64 {
    let mut end = b;
    while end > a && clusters[end - 1].is_space {
        end -= 1;
    }
    let widths: f64 = clusters[a..end].iter().map(|c| c.width).sum();
    // §23.4 tracking: one gap per boundary between consecutive
    // NON-ABSORBED clusters, owned by the logically-earlier one and
    // SUPPRESSED when the later side opens a join context (C-5
    // slice 2: ligature-interior boundaries have no gap at all;
    // UNSAFE_TO_BREAK boundaries keep the gap but not the tracking).
    let mut tracking = 0.0;
    let mut prev_real: Option<usize> = None;
    for i in a..end {
        if clusters[i].absorbed {
            continue;
        }
        if let Some(p) = prev_real {
            if !clusters[i].join_before {
                tracking += clusters[p].tracking_px;
            }
        }
        prev_real = Some(i);
    }
    widths + tracking
}

// ---------------------------------------------------------------------
// Line breaking (§23.3 [NORMATIVE v1] greedy first-fit)
// ---------------------------------------------------------------------

/// Break clusters into lines. `opportunities` are UAX #14 byte offsets
/// (provider truth); breaks happen only where an opportunity coincides
/// with a cluster boundary (§23.3). `measures[k]` is the measure of
/// line k (last entry repeats — first-line indent shortens line 0).
fn break_lines(
    clusters: &[Cluster],
    opportunities: &[usize],
    measures: &[f64],
) -> Vec<Range<usize>> {
    use std::collections::BTreeSet;
    let opps: BTreeSet<usize> = opportunities.iter().copied().collect();
    let n = clusters.len();
    let mut lines = Vec::new();
    let mut start = 0usize;
    while start < n {
        let measure = *measures
            .get(lines.len())
            .or(measures.last())
            .unwrap_or(&f64::INFINITY);
        // NOTE (perf debt, recorded): measure_width re-sums from the
        // line start per candidate — O(L²) per line. Fine at document
        // scale for v1; a prefix-sum pass is the CP-1.0 perf-budget fix.
        let mut best: Option<usize> = None; // cluster index AFTER which we break
        let mut i = start;
        let mut end_choice: Option<usize> = None;
        while i < n {
            let fits = measure_width(clusters, start, i + 1) <= measure + 1e-9;
            if !fits {
                // No further cluster joins this line. Prefer the last
                // UAX #14 opportunity; with NO opportunity on the line,
                // EMERGENCY-FILL (pinned interpretation, module
                // header): extend to the last FITTING cluster — §23.3
                // only sanctions mid-word breaks for the
                // single-cluster-wider-than-measure case, so the
                // emergency break carries as much of the fragment as
                // fits rather than one cluster at a time (review
                // finding, C-1 round: "AAAAAA" at measure 3·A must lay
                // out AAA|AAA, not A|A|A|AAA).
                end_choice = Some(best.unwrap_or(if i == start {
                    start + 1 // single cluster wider than the measure
                } else {
                    i
                }));
                break;
            }
            if clusters[i].mandatory_after {
                // LF always breaks; it never carries to the next line.
                end_choice = Some(i + 1);
                break;
            }
            if opps.contains(&clusters[i].range.end) {
                best = Some(i + 1);
            }
            i += 1;
        }
        let end = end_choice.unwrap_or(n); // everything fits to the end
        debug_assert!(end > start);
        lines.push(start..end);
        start = end;
    }
    if lines.is_empty() {
        lines.push(0..0);
    }
    lines
}

// ---------------------------------------------------------------------
// Justification (§23.3 steps 1–3)
// ---------------------------------------------------------------------

/// Per-line spread: a uniform expansion RATIO on each space cluster's
/// natural width (§23.3 step 1 "expand uniformly, up to +200 % of
/// natural width" — per-space, so a narrow space can never exceed
/// +200 % of ITS OWN width while wide ones absorb the aggregate;
/// review finding, C-1 round), plus a fixed per-gap letter-spacing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Spread {
    space_ratio: f64,
    per_gap: f64,
}

fn justify_spread(clusters: &[Cluster], line: &Range<usize>, deficit: f64) -> Spread {
    if deficit <= 0.0 || line.is_empty() {
        return Spread::default();
    }
    // Hanging trailing spaces take no spread.
    let mut end = line.end;
    while end > line.start && clusters[end - 1].is_space {
        end -= 1;
    }
    let body = &clusters[line.start..end];
    if body.is_empty() {
        return Spread::default();
    }
    let natural: f64 = body.iter().filter(|c| c.is_space).map(|c| c.width).sum();
    // Gaps count only boundaries between NON-ABSORBED clusters
    // (ligature-interior graphemes expose no boundary; C-5 slice 2).
    let gaps = body
        .iter()
        .filter(|c| !c.absorbed)
        .count()
        .saturating_sub(1);
    if natural == 0.0 {
        // Single word: ≤ +5 % em per gap, else start-aligned (no spread).
        if gaps == 0 {
            return Spread::default();
        }
        let em = body.iter().map(|c| c.em_px).fold(0.0, f64::max);
        let cap = 0.05 * em;
        let per_gap = deficit / gaps as f64;
        return if per_gap <= cap {
            Spread {
                space_ratio: 0.0,
                per_gap,
            }
        } else {
            Spread::default()
        };
    }
    // Step 1: a uniform expansion RATIO capped at +200 %: every space
    // grows by ratio × its own natural width, so the per-space cap and
    // the uniformity requirement hold simultaneously.
    let ratio = (deficit / natural).min(2.0);
    let mut left = deficit - ratio * natural;
    // Step 2: uniform letter-spacing, clamped to ±2 % em per gap;
    // step 3: render at the cap (loose line; margins never overflowed).
    let mut per_gap = 0.0;
    if left > 0.0 && gaps > 0 {
        let em = body.iter().map(|c| c.em_px).fold(0.0, f64::max);
        per_gap = (left / gaps as f64).min(0.02 * em);
        left -= per_gap * gaps as f64;
        let _ = left; // step 3: remaining deficit stays (loose line)
    }
    Spread {
        space_ratio: ratio,
        per_gap,
    }
}

// ---------------------------------------------------------------------
// Assembly
// ---------------------------------------------------------------------

/// Line leading per §23.3: auto = max(autoLeadingPct × size, a+d+gap)
/// over the line's runs (mixed sizes: max each part); fixed overrides.
fn line_metrics(
    clusters: &[Cluster],
    items: &[StyledItem],
    line: &Range<usize>,
    style: &ParagraphStyle,
) -> (f64, f64, f64) {
    let (mut ascent, mut descent, mut leading) = (0.0f64, 0.0f64, 0.0f64);
    let mut seen_any = false;
    for c in &clusters[line.clone()] {
        if let Some(si) = items.get(c.item_index) {
            seen_any = true;
            ascent = ascent.max(si.metrics.ascent);
            descent = descent.max(si.metrics.descent);
            let auto = (style.auto_leading_pct * si.em_px)
                .max(si.metrics.ascent + si.metrics.descent + si.metrics.line_gap);
            leading = leading.max(auto);
        }
    }
    if !seen_any {
        // Empty line (e.g. between two LFs): first item's metrics rule.
        if let Some(si) = items.first() {
            ascent = si.metrics.ascent;
            descent = si.metrics.descent;
            leading =
                (style.auto_leading_pct * si.em_px).max(ascent + descent + si.metrics.line_gap);
        }
    }
    if let Leading::Fixed(l) = style.leading {
        leading = l;
    }
    (ascent, descent, leading)
}

/// Place one line's glyphs. Clusters walk in VISUAL order (items via
/// UAX #9 L2 [`visual_order`], clusters within an item by buffer order —
/// RTL items' glyph buffers are already visual). Returns glyphs and the
/// natural width consumed.
fn place_line(
    clusters: &[Cluster],
    items: &[StyledItem],
    line: &Range<usize>,
    x_origin: f64,
    baseline: f64,
    spread: Spread,
) -> Vec<PlacedGlyph> {
    // Sub-items overlapping the line, in logical order.
    let line_bytes = clusters[line.start].range.start..clusters[line.end - 1].range.end;
    let mut subs: Vec<(usize, Item)> = Vec::new(); // (items index, clipped item)
    for (ii, si) in items.iter().enumerate() {
        let s = si.item.range.start.max(line_bytes.start);
        let e = si.item.range.end.min(line_bytes.end);
        if s < e {
            let mut it = si.item.clone();
            it.range = s..e;
            subs.push((ii, it));
        }
    }
    let order = visual_order(&subs.iter().map(|(_, it)| it.clone()).collect::<Vec<_>>());

    // Spread bookkeeping per cluster INDEX (byte→index via binary
    // search on the sorted cluster starts).
    let starts: Vec<usize> = clusters.iter().map(|c| c.range.start).collect();
    let cluster_of = |byte: usize| -> usize {
        match starts.binary_search(&byte) {
            Ok(i) => i,
            Err(i) => i - 1,
        }
    };
    let mut body_end = line.end;
    while body_end > line.start && clusters[body_end - 1].is_space {
        body_end -= 1;
    }
    let in_body = |ci: usize| (line.start..body_end).contains(&ci);

    // GAP ATTRIBUTION (C-5 slice 2 — the bidi-level-aware ownership
    // that closes the C-4 pinned residual): gaps are the n−1 spaces
    // between visually adjacent NON-ABSORBED body clusters. Ownership
    // PLAN, computed before the walk: (a) logically-adjacent visual
    // pairs (in the absorbed-filtered logical sequence) take the
    // logically-earlier cluster's tracking; (b) direction-edge visual
    // pairs (not logically adjacent) take the UNPLACED logical gaps,
    // matched in order (deferred transitions in visual order ↔
    // unassigned filtered-logical gaps in logical order) — for any
    // single- or multi-segment embedding this reproduces
    // measure_width's logical sum exactly, so width == ink for ANY
    // style/direction mix (the C-4 residual's recorded probe is the
    // oracle). PIN (review D3): this is a TOTALS guarantee; at
    // direction edges the individual amounts are a permutation
    // relative to the per-advance reading (each displaced amount stays
    // adjacent to one endpoint of its logical gap — no canonical
    // placement exists for tracking at bidi edges; register line). Tracking is SUPPRESSED where the owned gap's later
    // cluster opens a join context (§23.4); justification per_gap
    // keeps its §23.3 authority at every real boundary. Space-cluster
    // width expansion rides the cluster itself, direction-free.
    //
    // Prepass 1: the visual sequence of non-absorbed body clusters,
    // derived by the SAME iteration the walk performs.
    let mut visual_seq: Vec<usize> = Vec::new();
    {
        let mut cur: Option<usize> = None;
        for &oi in &order {
            let (_, ref sub) = subs[oi];
            let si = &items[subs[oi].0];
            for g in si.glyphs.iter().filter(|g| sub.range.contains(&g.cluster)) {
                let ci = cluster_of(g.cluster);
                if cur != Some(ci) {
                    cur = Some(ci);
                    if in_body(ci) && !clusters[ci].absorbed {
                        visual_seq.push(ci);
                    }
                }
            }
        }
    }
    // Prepass 2: filtered logical sequence + the ownership plan.
    let real: Vec<usize> = (line.start..body_end)
        .filter(|&ci| !clusters[ci].absorbed)
        .collect();
    let pos_of = |ci: usize| real.binary_search(&ci).ok();
    let gap_tracking = |k: usize| -> f64 {
        // Filtered-logical gap k = between real[k] and real[k+1];
        // suppressed when the later side opens a join context.
        if clusters[real[k + 1]].join_before {
            0.0
        } else {
            clusters[real[k]].tracking_px
        }
    };
    let n_real = real.len();
    let mut assigned = vec![false; n_real.saturating_sub(1)];
    let mut plan: Vec<Option<f64>> = Vec::new(); // per visual transition
    let mut deferred: Vec<usize> = Vec::new(); // transition ordinals
    for w in visual_seq.windows(2) {
        let (pa, pb) = (pos_of(w[0]), pos_of(w[1]));
        match (pa, pb) {
            (Some(pa), Some(pb)) if pa.abs_diff(pb) == 1 => {
                let k = pa.min(pb);
                assigned[k] = true;
                plan.push(Some(gap_tracking(k)));
            }
            _ => {
                deferred.push(plan.len());
                plan.push(None);
            }
        }
    }
    let unassigned: Vec<usize> = (0..n_real.saturating_sub(1))
        .filter(|&k| !assigned[k])
        .collect();
    debug_assert_eq!(deferred.len(), unassigned.len(), "gap bijection");
    for (slot, k) in deferred.into_iter().zip(unassigned) {
        plan[slot] = Some(gap_tracking(k));
    }

    let mut out = Vec::new();
    let mut pen = x_origin;
    let mut prev_body_cluster: Option<usize> = None;
    let mut current_cluster: Option<usize> = None;
    let mut transition = 0usize;
    for &oi in &order {
        let (ii, ref sub) = subs[oi];
        let si = &items[ii];
        // Glyphs of this sub-item, buffer order, filtered to the range.
        // A cluster's spread applies ONCE, at the transition to the
        // next grapheme cluster — never per glyph, so multi-glyph
        // clusters (base+mark under cluster level 1) cannot double it
        // (review finding, C-1 round). `current_cluster` guards the
        // entry transition the same way (a second glyph of the same
        // cluster must not re-trigger the gap).
        let mut it = si
            .glyphs
            .iter()
            .filter(|g| sub.range.contains(&g.cluster))
            .peekable();
        while let Some(g) = it.next() {
            let ci = cluster_of(g.cluster);
            // Entering a NEW non-absorbed body cluster: settle the
            // planned gap for this visual transition.
            if current_cluster != Some(ci) {
                current_cluster = Some(ci);
                if in_body(ci) && !clusters[ci].absorbed && prev_body_cluster.is_some() {
                    let tracked = plan.get(transition).copied().flatten().unwrap_or(0.0);
                    pen += spread.per_gap + tracked;
                    transition += 1;
                }
            }
            out.push(PlacedGlyph {
                glyph: g.glyph,
                face: si.item.face,
                cluster: g.cluster,
                x: pen + g.offset_x,
                y: baseline - g.offset_y, // HB y-up → doc y-down
                advance: g.advance,
            });
            pen += g.advance;
            let next_same = it.peek().is_some_and(|h| cluster_of(h.cluster) == ci);
            if !next_same {
                // Cluster complete: apply its own width expansion.
                if in_body(ci) && clusters[ci].is_space {
                    pen += clusters[ci].width * spread.space_ratio;
                }
                if in_body(ci) {
                    prev_body_cluster = Some(ci);
                }
            }
        }
    }
    out
}

/// Line x origin from alignment (§23.3; start/end mirror under RTL).
fn align_origin(
    align: Align,
    is_last: bool,
    justify_last: JustifyLast,
    rtl: bool,
    left: f64,
    avail: f64,
    natural: f64,
) -> (f64, bool) {
    // Returns (x_origin, justify_this_line).
    let a = if align == Align::Justify {
        if is_last {
            match justify_last {
                JustifyLast::Start => Align::Start,
                JustifyLast::Center => Align::Center,
                JustifyLast::End => Align::End,
                JustifyLast::Full => return (left, true),
            }
        } else {
            return (left, true);
        }
    } else {
        align
    };
    let x = match (a, rtl) {
        (Align::Start, false) | (Align::End, true) => left,
        (Align::End, false) | (Align::Start, true) => left + avail - natural,
        (Align::Center, _) => left + (avail - natural) / 2.0,
        (Align::Justify, _) => left,
    };
    (x, false)
}

/// §23.3 paragraph/area text layout for ONE paragraph inside `box_`,
/// stacking from `top` (caller adds spaceBefore per the §23.3
/// adjacent-adds rule). Inputs come straight from the pipeline seams:
/// grapheme `bounds`, UAX #14 `opportunities`, shaped `items`.
#[allow(clippy::too_many_arguments)]
pub fn layout_paragraph(
    text: &str,
    items: &[StyledItem],
    bounds: &[usize],
    opportunities: &[usize],
    style: &ParagraphStyle,
    box_: &LayoutBox,
    top: f64,
) -> ParagraphLayout {
    let clusters = measure_clusters(text, items, bounds);
    let inner_left = box_.left + style.indent_left;
    let inner_right = box_.right - style.indent_right;
    let measure = (inner_right - inner_left).max(0.0);
    // First-line indent applies at the LEADING edge (§23.3: "the whole
    // scheme mirrors under RTL"), so a negative (hanging) value clamps
    // against the leading-edge indent ONLY — indent_left under LTR,
    // indent_right under RTL. (Review finding, C-1 round: the earlier
    // direction-agnostic min() erased the classic hanging indent
    // whenever the trailing-edge indent was 0.)
    let leading_indent = if style.rtl_base {
        style.indent_right
    } else {
        style.indent_left
    };
    let first_indent = style.indent_first.max(-leading_indent);
    let first_measure = (measure - first_indent).max(0.0);
    let measures = [first_measure, measure];

    let line_ranges = break_lines(&clusters, opportunities, &measures);

    let mut lines = Vec::new();
    let mut y = top;
    let mut overflowed = false;
    let mut first_hidden = None;
    let mut bottom = top;
    for (k, lr) in line_ranges.iter().enumerate() {
        if lr.is_empty() {
            continue;
        }
        let (ascent, descent, leading) = line_metrics(&clusters, items, lr, style);
        let baseline = if lines.is_empty() {
            y + ascent // first baseline: box.top (+stack) + ascent
        } else {
            y + leading // later baselines step by the LINE's leading
        };
        if baseline + descent > box_.bottom + 1e-9 {
            overflowed = true;
            first_hidden = Some(clusters[lr.start].range.start);
            break;
        }
        let is_last = k == line_ranges.len() - 1 || clusters[lr.end - 1].mandatory_after;
        let natural = measure_width(&clusters, lr.start, lr.end);
        let (line_measure, line_left) = if k == 0 {
            let extra = if style.rtl_base { 0.0 } else { first_indent };
            (first_measure, inner_left + extra)
        } else {
            (measure, inner_left)
        };
        let (x0, justify) = align_origin(
            style.align,
            is_last,
            style.justify_last,
            style.rtl_base,
            line_left,
            line_measure,
            natural,
        );
        let spread = if justify {
            justify_spread(&clusters, lr, line_measure - natural)
        } else {
            Spread::default()
        };
        let glyphs = place_line(&clusters, items, lr, x0, baseline, spread);
        bottom = baseline + descent;
        lines.push(LaidLine {
            range: clusters[lr.start].range.start..clusters[lr.end - 1].range.end,
            baseline,
            width: natural,
            glyphs,
        });
        y = baseline;
    }
    ParagraphLayout {
        lines,
        overflowed,
        first_hidden,
        bottom,
    }
}

/// §23.3 point text: lines break ONLY on explicit newlines; the anchor
/// lies on the first baseline; alignment is literal left/center/right
/// about the anchor (pinned interpretation, module header).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointAlign {
    Left,
    Center,
    Right,
}

pub fn layout_point_text(
    text: &str,
    items: &[StyledItem],
    bounds: &[usize],
    style: &ParagraphStyle,
    anchor: (f64, f64),
    align: PointAlign,
) -> ParagraphLayout {
    let clusters = measure_clusters(text, items, bounds);
    // Lines = mandatory breaks only.
    let mut line_ranges: Vec<Range<usize>> = Vec::new();
    let mut start = 0;
    for (i, c) in clusters.iter().enumerate() {
        if c.mandatory_after {
            line_ranges.push(start..i + 1);
            start = i + 1;
        }
    }
    if start < clusters.len() {
        line_ranges.push(start..clusters.len());
    }
    let mut lines = Vec::new();
    let mut baseline = anchor.1;
    let mut bottom = anchor.1;
    for (k, lr) in line_ranges.iter().enumerate() {
        if lr.is_empty() {
            continue;
        }
        let (_, descent, leading) = line_metrics(&clusters, items, lr, style);
        if k > 0 {
            baseline += leading;
        }
        let natural = measure_width(&clusters, lr.start, lr.end);
        let x0 = match align {
            PointAlign::Left => anchor.0,
            PointAlign::Center => anchor.0 - natural / 2.0,
            PointAlign::Right => anchor.0 - natural,
        };
        let glyphs = place_line(&clusters, items, lr, x0, baseline, Spread::default());
        bottom = baseline + descent;
        lines.push(LaidLine {
            range: clusters[lr.start].range.start..clusters[lr.end - 1].range.end,
            baseline,
            width: natural,
            glyphs,
        });
    }
    ParagraphLayout {
        lines,
        overflowed: false,
        first_hidden: None,
        bottom,
    }
}

// ---------------------------------------------------------------------
// Text on a path (§23.3)
// ---------------------------------------------------------------------

/// On-path parameters (§23.3 table: side/offset/start/end).
#[derive(Clone, Copy, Debug)]
pub struct OnPathParams {
    /// Arc-length window start (px along the path).
    pub start: f64,
    /// Arc-length window end.
    pub end: f64,
    /// Signed distance along the normal (baseline shift folds in here
    /// at C-2 when the style model lands).
    pub offset: f64,
    /// Mirror glyphs to the other side of the path.
    pub flip_side: bool,
}

/// One on-path placed glyph: baseline position + tangent angle
/// (radians; the baseline follows the tangent, §23.3).
#[derive(Clone, Debug, PartialEq)]
pub struct OnPathGlyph {
    pub glyph: u32,
    pub face: FaceId,
    pub cluster: usize,
    pub pos: Vec2,
    pub angle: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OnPathLayout {
    pub glyphs: Vec<OnPathGlyph>,
    /// Text exceeded the [start, end] window (§23.3: the remainder is
    /// not rendered; the overlay shows the indicator at the end anchor).
    pub overflowed: bool,
}

/// Left normal of a unit tangent in y-down doc space (the "up" side of
/// a left-to-right path); `flip_side` mirrors it.
fn normal_of(t: Vec2, flip: bool) -> Vec2 {
    let n = Vec2::new(t.y, -t.x);
    if flip {
        -n
    } else {
        n
    }
}

/// §23.3 on-path placement: the pseudocode verbatim, clusters in visual
/// order, plus the pinned tight-curvature stretch realization (module
/// header). Single-line semantics (mandatory breaks don't apply on a
/// path).
pub fn layout_on_path(
    text: &str,
    items: &[StyledItem],
    bounds: &[usize],
    table: &ArcLengthTable,
    params: &OnPathParams,
) -> OnPathLayout {
    let clusters = measure_clusters(text, items, bounds);
    let window_end = params.end.min(table.total());
    let mut out = Vec::new();
    let mut overflowed = false;

    // Visual order across the whole text (one line).
    let all: Vec<Item> = items.iter().map(|s| s.item.clone()).collect();
    let order = visual_order(&all);

    let mut s = params.start;
    'outer: for &oi in &order {
        let si = &items[oi];
        // Clusters of this item in buffer (visual) order: walk glyphs
        // and group by cluster start.
        let mut k = 0usize;
        while k < si.glyphs.len() {
            let cstart = si.glyphs[k].cluster;
            // (perf debt, recorded: linear find per cluster — a shared
            // byte→cluster index joins the CP-1.0 perf pass.)
            let cluster = clusters
                .iter()
                .find(|c| c.range.contains(&cstart))
                .expect("glyph cluster inside bounds");
            let mut advance = 0.0;
            let mut glyphs_here = Vec::new();
            while k < si.glyphs.len() && cluster.range.contains(&si.glyphs[k].cluster) {
                glyphs_here.push(&si.glyphs[k]);
                advance += si.glyphs[k].advance;
                k += 1;
            }
            if cluster.mandatory_after {
                continue; // no newlines on a path
            }
            // Tight-curvature stretch (pinned realization): radius at
            // s_mid from tangent turn over the cluster's span.
            let s_mid_probe = s + advance / 2.0;
            let stretch = {
                let h = (advance / 2.0).max(1e-6);
                match (
                    table.tangent_at_length((s_mid_probe - h).max(0.0)),
                    table.tangent_at_length((s_mid_probe + h).min(table.total())),
                ) {
                    (Some(t1), Some(t2)) => {
                        let dot = t1.dot(t2).clamp(-1.0, 1.0);
                        let dtheta = dot.acos();
                        if dtheta > 1e-12 {
                            let radius = (2.0 * h) / dtheta;
                            let kappa_sign = t1.cross(t2).signum();
                            // Outer side: glyph side opposite the
                            // curvature center (baseline-on-path = outer).
                            // The center lies at −κ_sign·n̂, so a glyph
                            // at normal offset `side` is outer iff
                            // side·κ_sign ≥ 0.
                            let side = if params.flip_side {
                                -params.offset
                            } else {
                                params.offset
                            };
                            let outer = side * kappa_sign >= 0.0;
                            if radius < cluster.em_px && outer {
                                (cluster.em_px / radius).min(1.25)
                            } else {
                                1.0
                            }
                        } else {
                            1.0
                        }
                    }
                    _ => 1.0,
                }
            };
            let s_mid = s + advance / 2.0;
            if s_mid > window_end || s + advance * stretch > window_end {
                overflowed = true;
                break 'outer;
            }
            let (Some(p), Some(t)) = (table.point_at_length(s_mid), table.tangent_at_length(s_mid))
            else {
                overflowed = true;
                break 'outer;
            };
            let n = normal_of(t, params.flip_side);
            let base = p + n * params.offset;
            let angle = t.y.atan2(t.x);
            // Within the cluster, glyphs place along the tangent
            // relative to the cluster midpoint (`base` sits at s_mid);
            // HB offsets rotate into the tangent frame (offset_y along
            // the normal).
            let mut pen = 0.0;
            for g in glyphs_here {
                let along = pen - advance / 2.0;
                let gpos = base + t * (along + g.offset_x) + n * g.offset_y;
                out.push(OnPathGlyph {
                    glyph: g.glyph,
                    face: si.item.face,
                    cluster: g.cluster,
                    pos: gpos,
                    angle,
                });
                pen += g.advance;
            }
            s += advance * stretch;
        }
    }
    OnPathLayout {
        glyphs: out,
        overflowed,
    }
}
