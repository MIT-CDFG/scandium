// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-algo` — algorithm ports staged per the ranked census order.
//!
//! Spec: ES Part 45 (§45.1 heal core, §45.3 deterministic exemplar
//! synthesis); ALGORITHMS.md areas per module; ports strictly per
//! `appname-proto/research/code-census/INTEGRATION-NOTES.md` §6 ranked
//! order; rights per `contracts/algorithms/{algorithms,patent-register}.yaml`.
//!
//! Every engine here is a PURE function over planar rasters ([`plane::Plane`])
//! — no document access, no side-door mutation APIs. Consumers (the
//! appname-tools filter corpus, appname-core op registry) adapt tile data at
//! brokered adapter seams and route every mutation through the
//! `appname.command.v1` envelope spine (D-48). Determinism: single-threaded
//! fixed-order scalar code; fixed-seed oracles in every module's tests.
//!
//! Wave-1 modules (INTEGRATION-NOTES §6 order 1–7):
//!  1. [`sparse`] + [`matting`] — pymatting port (MIT): closed-form matting
//!     Laplacian, ichol-preconditioned CG, foreground estimation.
//!  2. [`fusion`] — exposure-fusion port (BSD-2): Mertens weights +
//!     Laplacian-pyramid fusion.
//!  3. [`pigment`] — pigment stack: rgb2spec LUT bake (BSD-3), spectral.js
//!     Kubelka–Munk cross-check (MIT), `.myb` preset importer (ISC memo).
//!  4. [`inpaint`] — boundary-diffusion fill fallback (telea04 FMM contract).
//!  5. [`poisson`] — healing Poisson core (perez03; fpie-census structure).
//!  6. [`deconv`] — RL/Wiener (scikit-image contract) + hyper-Laplacian
//!     deconvolution (krishnan09, clean-room from paper).
//!  7. [`color_transfer`] — Reinhard lαβ statistics transfer (reinhard01).
//!
//! Wave-2 modules (INTEGRATION-NOTES §6 order 8–…):
//!  8. [`graphcut`] + [`grabcut`] — own-written push-relabel min-cut/max-flow
//!     (goldberg88 era-expired lineage; ortools-maxflow census slot) +
//!     rother04/boykov01 iterated-GMM graph-cut orchestration
//!     (`legacy_selection.grabcut`, the wand/quick-select ML-free tier).
//!  9. [`mls`] — moving-least-squares deformation (schaefer06,
//!     clean-room from paper; census cross-check repo not consulted):
//!     affine/similarity/rigid point-handle variants, displacement
//!     lattice + dense reference, backward-map plane warp
//!     (`warps_upright` MLS core; `transform.puppet` rigid tier).
//! 10. [`edge_aware`] — edge-aware detail family (clean-room from the
//!     papers; area author codes are no-grant and ximgproc holds no LLF —
//!     census route): guided filter (he10) + Shadows/Highlights split,
//!     domain transform NC/RF (gastal11), WLS decomposition (farbman08,
//!     on [`sparse`]), and the fast/exact local Laplacian pair
//!     (aubry14 + paris11) — the area's L2 FTO-flagship primary.
//! 11. [`aces`] — ACES 2.0 output transforms (Apache-2.0 translation port
//!     from the Academy's CTL library at the census pin): tonescale, JMh
//!     appearance pipeline, chroma + gamut compression, SDR sRGB/BT.1886
//!     display encoding (`hdr_tonemap` `tmo_set` `aces_view_transform`).
//! 12. [`pano`] — pano classical tier (clean-room from the papers;
//!     census route: the upstream detector library is a non-staged
//!     behavior reference, nothing fetched): scale-invariant features
//!     (lowe04; patent P-6711293 verified "Expired - Lifetime"
//!     2020-03-06), oriented binary features (rublee11), ratio-test
//!     matching, normalized-DLT + seeded-RANSAC homography (hartley04),
//!     Burt–Adelson multiband seam blending (burt83) on the [`fusion`]
//!     pyramid substrate. Bundle adjustment stays a LINKED dependency
//!     (Part 48 record draft, item 12b — no solver code in this crate).
//!
//! CP-1.0 wave modules:
//! 13. [`exemplar`] — the ES §45.3 deterministic exemplar synthesis
//!     engine (content-aware fill core; also backs content-aware move
//!     and the spot-heal exemplar tier). Clean-room from criminisi04 /
//!     ashikhmin01 / hertzmann01 / kwatra03 (census key
//!     `caf_primary_cleanroom`, verdict none-found BY DESIGN); the
//!     contract's three structural invariants are enforced
//!     architecturally and locked by mechanical + behavioral tests;
//!     seams reuse the wave-2 [`graphcut`] engine; the boundary-
//!     diffusion degradation path is §45.3 failure rule 1.
//! 14. [`matting`] row-B completion (Select & Mask matting engines
//!     beyond the wave-1 closed-form core): [`matting::knn`] (chen12
//!     KNN quality option, pymatting port), [`matting::guided_tier`]
//!     (he10 + he15fast interactive tier over [`edge_aware::guided`]),
//!     [`matting::decontaminate`] (ES §13.4 stage-8 decontamination
//!     over the wave-1 foreground estimator), [`matting::fallback`]
//!     (the contract's `guided_filter_plus_knn` revival fallback).
//! 15. [`lensdb`] — Row C: own-Rust lens profile database reader
//!     (contract `lens_correction.database` =
//!     `lensfun_db_own_rust_reader`, `library_linked: false`, data
//!     license CC-BY-SA-3.0): minimal XML subset parser, profile
//!     model, user-replaceable-dir reader, deterministic lookup +
//!     calibration interpolation feeding O-IO's RAW develop
//!     `lens_correction` stage (ES Part 43 §43.3 sub-stage 4b;
//!     ALGORITHMS.md §21). Data staging is O-BUILD's; no lens-
//!     correction library code consulted (clean-room rule).

pub mod aces;
pub mod color_transfer;
pub mod deconv;
pub mod edge_aware;
pub mod exemplar;
pub mod fft;
pub mod fusion;
pub mod grabcut;
pub mod graphcut;
pub mod inpaint;
pub mod lensdb;
pub mod matting;
pub mod mls;
pub mod pano;
pub mod pigment;
pub mod plane;
pub mod poisson;
/// ★ NOTE the deliberate name split: [`rgba_blur::RgbaPlane`] is the
/// 4-channel premultiplied carrier the §16.3/§11.2 Gaussian works on, and
/// is a DIFFERENT type from the single-channel [`plane::Plane`] above.
/// Never merged, never shadowed.
pub mod rgba_blur;
pub mod sparse;

/// Crate name marker exercised by the bootstrap smoke test.
pub const CRATE_NAME: &str = "appname-algo";

/// cfg(test)-only allocation probe: a counting wrapper around the system
/// allocator with a PER-THREAD byte counter, used by allocation-
/// discipline regression tests (e.g. the exemplar refine path's
/// no-per-patch-canvas-clones evidence). Per-thread counting keeps the
/// measurements exact under the default multi-threaded test harness.
/// Counts are monotonic allocation TRAFFIC (frees are not subtracted).
#[cfg(test)]
pub(crate) mod alloc_probe {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static ALLOCATED: Cell<u64> = const { Cell::new(0) };
    }

    #[inline]
    fn bump(n: u64) {
        // try_with: the allocator may run during thread teardown after
        // this TLS slot is destroyed; those bytes are simply not counted.
        let _ = ALLOCATED.try_with(|c| c.set(c.get() + n));
    }

    /// Heap bytes requested by THIS thread so far (monotonic).
    pub fn allocated_on_this_thread() -> u64 {
        ALLOCATED.with(|c| c.get())
    }

    struct CountingAlloc;

    // SAFETY: delegates every operation unchanged to `System`; the only
    // addition is a TLS counter update, which never allocates (const-
    // initialized Cell) and never touches the returned pointers.
    unsafe impl GlobalAlloc for CountingAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            bump(layout.size() as u64);
            unsafe { System.alloc(layout) }
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            bump(layout.size() as u64);
            unsafe { System.alloc_zeroed(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            bump((new_size as u64).saturating_sub(layout.size() as u64));
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    #[global_allocator]
    static GLOBAL: CountingAlloc = CountingAlloc;
}

#[cfg(test)]
mod tests {
    /// Bootstrap smoke test: proves the crate compiles and `cargo test`
    /// exercises it.
    #[test]
    fn bootstrap_smoke() {
        assert_eq!(super::CRATE_NAME, "appname-algo");
    }
}
