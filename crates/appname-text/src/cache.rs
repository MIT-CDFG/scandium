// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Glyph raster cache (CP-1.0 slice C-3).
//!
//! Spec: ES §23.2 "Cache" [NORMATIVE]: rasters keyed by **(face, em
//! size, variation coords, hint mode, phase, glyph id)**, LRU under the
//! tile-cache budget (§30.1). The em-size component uses the same 26.6
//! fixed-point integer the rasterizer and shaper share (exact — no f64
//! key wobble); subpixel `phase` is the ¼-px x-quantization of §23.2
//! (grayscale default renders phase 0; the preference arrives with the
//! render binding). Variation coords are carried in the key NOW so
//! §44.3's "glyph rasters key on the coordinate tuple" holds the day
//! the rasterizer grows variation support — until then callers pass an
//! empty list (honest limit; the field is not dead weight, it is the
//! §44.3 contract surface).
//!
//! Determinism: the cache changes WHEN rasters are computed, never
//! what they contain — eviction can only cause recomputation, and the
//! rasterizer is deterministic (§23.2). LRU order itself is
//! deterministic (BTreeMap + monotonic ticks; no hash iteration).

use std::collections::BTreeMap;
use std::rc::Rc;

use crate::pipeline::{FaceId, GlyphRasterizer};
use crate::raster::GlyphCoverage;

/// Hint mode as keyed by §23.2 (derived from the em/translation rule by
/// the caller — the rule needs the layer transform, which the cache
/// cannot know).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HintMode {
    AutohintLight,
    Unhinted,
}

/// §23.2 cache key. Variation coords as (tag, 16.16 fixed) — exact,
/// order-significant (callers pass the run's `variations` order; §44.3
/// stores user-space values, quantized here only for key equality).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RasterKey {
    pub face: FaceId,
    /// em in 26.6 fixed point (the shared ×64 convention).
    pub em_26_6: u32,
    pub variations: Vec<([u8; 4], i32)>,
    pub hint: HintMode,
    /// ¼-px subpixel phase, 0–3 (§23.2; grayscale default = 0).
    pub phase: u8,
    pub glyph: u32,
}

struct Entry {
    coverage: Rc<GlyphCoverage>,
    bytes: usize,
    tick: u64,
}

/// LRU raster cache under a byte budget (§30.1: the text raster cache
/// lives under the tile-cache budget; the budget VALUE is wired by the
/// engine integration — this type enforces whatever it is given).
pub struct RasterCache {
    entries: BTreeMap<RasterKey, Entry>,
    budget_bytes: usize,
    used_bytes: usize,
    tick: u64,
    hits: u64,
    misses: u64,
}

impl RasterCache {
    pub fn new(budget_bytes: usize) -> Self {
        RasterCache {
            entries: BTreeMap::new(),
            budget_bytes,
            used_bytes: 0,
            tick: 0,
            hits: 0,
            misses: 0,
        }
    }

    pub fn stats(&self) -> (u64, u64, usize) {
        (self.hits, self.misses, self.used_bytes)
    }

    /// Fetch or compute the raster for `key`, rastering through `r` on
    /// miss. The key's em and the raster em agree BY CONSTRUCTION: the
    /// cache derives `em_px` from the key's 26.6 field and passes it to
    /// the rasterizer itself — no caller convention to drift.
    pub fn get_or_raster<R>(&mut self, r: &R, key: &RasterKey) -> Rc<GlyphCoverage>
    where
        R: GlyphRasterizer<Coverage = GlyphCoverage>,
    {
        self.tick += 1;
        // Hit path clones nothing (the variations Vec would otherwise
        // heap-allocate per lookup once variation support lands).
        if let Some(e) = self.entries.get_mut(key) {
            e.tick = self.tick;
            self.hits += 1;
            return Rc::clone(&e.coverage);
        }
        self.misses += 1;
        let em_px = f64::from(key.em_26_6) / 64.0;
        let coverage = Rc::new(r.raster(key.face, key.glyph, em_px));
        let bytes = coverage.data.len() * core::mem::size_of::<f32>()
            + core::mem::size_of::<GlyphCoverage>();
        // Insert, then evict least-recently-used until under budget —
        // the NEW entry survives even if alone over budget (a cache
        // that cannot hold the working glyph would thrash forever).
        self.entries.insert(
            key.clone(),
            Entry {
                coverage: Rc::clone(&coverage),
                bytes,
                tick: self.tick,
            },
        );
        self.used_bytes += bytes;
        while self.used_bytes > self.budget_bytes && self.entries.len() > 1 {
            let (oldest_key, _) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.tick)
                .map(|(k, e)| (k.clone(), e.tick))
                .expect("non-empty");
            if let Some(e) = self.entries.remove(&oldest_key) {
                self.used_bytes -= e.bytes;
            }
        }
        coverage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic fake rasterizer: coverage payload encodes the
    /// (glyph, em) pair; counts calls.
    struct CountingRaster(std::cell::Cell<u32>);
    impl GlyphRasterizer for CountingRaster {
        type Coverage = GlyphCoverage;
        fn raster(&self, _f: FaceId, glyph: u32, em_px: f64) -> GlyphCoverage {
            self.0.set(self.0.get() + 1);
            GlyphCoverage {
                left: glyph as i32,
                top: em_px as i32,
                width: 4,
                height: 1,
                data: vec![glyph as f32; 4],
            }
        }
    }

    fn key(glyph: u32, em_26_6: u32, hint: HintMode) -> RasterKey {
        RasterKey {
            face: FaceId(0),
            em_26_6,
            variations: Vec::new(),
            hint,
            phase: 0,
            glyph,
        }
    }

    #[test]
    fn hits_do_not_re_raster_and_key_components_distinguish() {
        let r = CountingRaster(std::cell::Cell::new(0));
        let mut c = RasterCache::new(1 << 20);
        let a1 = c.get_or_raster(&r, &key(1, 1024, HintMode::AutohintLight));
        let a2 = c.get_or_raster(&r, &key(1, 1024, HintMode::AutohintLight));
        assert_eq!(r.0.get(), 1, "second fetch is a hit");
        assert!(Rc::ptr_eq(&a1, &a2));
        // Different glyph, em, hint, phase each miss.
        c.get_or_raster(&r, &key(2, 1024, HintMode::AutohintLight));
        c.get_or_raster(&r, &key(1, 2048, HintMode::AutohintLight));
        c.get_or_raster(&r, &key(1, 1024, HintMode::Unhinted));
        let mut k = key(1, 1024, HintMode::AutohintLight);
        k.phase = 1;
        c.get_or_raster(&r, &k);
        assert_eq!(r.0.get(), 5);
        assert_eq!(c.stats().0, 1, "exactly one hit");
    }

    #[test]
    fn lru_evicts_oldest_under_byte_budget() {
        let r = CountingRaster(std::cell::Cell::new(0));
        // Entry ≈ 16 data bytes + struct size; budget for ~2 entries.
        let per = 4 * 4 + core::mem::size_of::<GlyphCoverage>();
        let mut c = RasterCache::new(2 * per);
        c.get_or_raster(&r, &key(1, 1024, HintMode::Unhinted));
        c.get_or_raster(&r, &key(2, 1024, HintMode::Unhinted));
        c.get_or_raster(&r, &key(1, 1024, HintMode::Unhinted)); // refresh 1
        c.get_or_raster(&r, &key(3, 1024, HintMode::Unhinted)); // evicts 2
        assert_eq!(r.0.get(), 3);
        c.get_or_raster(&r, &key(1, 1024, HintMode::Unhinted)); // still hit
        assert_eq!(r.0.get(), 3, "glyph 1 survived (refreshed before eviction)");
        c.get_or_raster(&r, &key(2, 1024, HintMode::Unhinted)); // re-raster
        assert_eq!(r.0.get(), 4, "glyph 2 was the LRU victim");
        assert!(c.stats().2 <= 2 * per + per, "byte accounting bounded");
    }

    #[test]
    fn over_budget_single_entry_survives() {
        let r = CountingRaster(std::cell::Cell::new(0));
        let mut c = RasterCache::new(1); // absurdly small
        let a = c.get_or_raster(&r, &key(1, 1024, HintMode::Unhinted));
        assert_eq!(a.data.len(), 4, "raster still served");
        let b = c.get_or_raster(&r, &key(2, 1024, HintMode::Unhinted));
        assert_eq!(b.left, 2);
        assert_eq!(r.0.get(), 2);
    }
}
