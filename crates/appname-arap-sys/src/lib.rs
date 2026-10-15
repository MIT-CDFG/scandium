// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-arap-sys` — the ARAP solve wrapper: Appname's FFI boundary
//! over the ruled libigl cpp-shim consumption (deps/libigl.yaml + the
//! O-INT 2026-08-17 ruling: file-level MPL via a shim TU compiled from
//! the PINNED EXTERNAL checkout; deps/eigen.yaml sanctioned shape
//! enforced by build.rs). D2 restructure (O-INT §5 fix-round ruling,
//! warps row): ES §1.3 [NORMATIVE] confines `unsafe` to `*-sys` crates —
//! the appname-shape-sys precedent — so the extern boundary moved here
//! from `appname-tools`, which now consumes only this crate's SAFE
//! value API.
//!
//! Design rules (shape-sys discipline):
//! * The public API is SAFE and value-based: triplets in,
//!   [`MqwfFactorization`] handle out, `Vec<f64>` solutions out. No raw
//!   pointer crosses the boundary.
//! * Every shim return is checked; C++ never receives a null/short
//!   buffer (lengths are enforced on the Rust side before the call).
//! * Determinism (D-48/§45.5): the shim's factorization is a fixed
//!   sequential sparse solve over caller-supplied triplets; nothing
//!   reads system state.
//! * Feature-gated whole: without `arap-libigl` this file compiles to
//!   an EMPTY crate (no extern block, no symbols, nothing to link) and
//!   build.rs no-ops — default builds carry zero libigl/Eigen surface.
//!
//! What the shim provides (see shim/arap_global_shim.cpp): the ARAP
//! include closure's named solver spine `igl::min_quad_with_fixed` —
//! factor-once per topology (§18.4) + per-RHS back-substitution. The
//! §45.5 solver SEMANTICS (local rotation fit, rotation pins, rigidity
//! smoothing, convergence) live spec-side in `appname_tools::puppet` —
//! this crate is transport, not math.

#![cfg_attr(feature = "arap-libigl", deny(unsafe_op_in_unsafe_fn))]

#[cfg(feature = "arap-libigl")]
mod ffi {
    use core::ffi::c_void;

    unsafe extern "C" {
        /// Factor the n×n SPD system given as COO triplets (i, j, v
        /// flattened f64 triples; i/j pre-validated < n Rust-side).
        /// Returns null on failure.
        pub fn appname_arap_mqwf_new(n: i32, triplets: *const f64, n_triplets: i32) -> *mut c_void;
        /// Back-substitute one RHS (length n) into `out` (length n).
        /// Returns 0 on success.
        pub fn appname_arap_mqwf_solve(handle: *mut c_void, b: *const f64, out: *mut f64) -> i32;
        pub fn appname_arap_mqwf_free(handle: *mut c_void);
    }
}

/// A factored sparse SPD system (safe owner of the shim handle).
#[cfg(feature = "arap-libigl")]
pub struct MqwfFactorization {
    handle: *mut core::ffi::c_void,
    n: usize,
}

// SAFETY: the handle is an owned, exclusively-held opaque object; the
// shim exposes no shared state, and every call requires `&self`/`&mut
// self` on the single owner (the §45.5 commit driver is
// single-threaded).
#[cfg(feature = "arap-libigl")]
unsafe impl Send for MqwfFactorization {}

#[cfg(feature = "arap-libigl")]
impl Drop for MqwfFactorization {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: handle came from appname_arap_mqwf_new and is
            // freed exactly once (self is being dropped).
            unsafe { ffi::appname_arap_mqwf_free(self.handle) };
        }
    }
}

#[cfg(feature = "arap-libigl")]
impl MqwfFactorization {
    /// Factor an `n × n` SPD system from `(i, j, v)` triplets. `None` on
    /// invalid indices, ABI-range overflow, or precompute failure.
    pub fn factor(n: usize, triplets: &[(u32, u32, f64)]) -> Option<MqwfFactorization> {
        let n_i32 = i32::try_from(n).ok()?;
        let count = i32::try_from(triplets.len()).ok()?;
        let mut flat = Vec::with_capacity(triplets.len() * 3);
        for &(i, j, v) in triplets {
            if i as usize >= n || j as usize >= n {
                return None;
            }
            flat.push(f64::from(i));
            flat.push(f64::from(j));
            flat.push(v);
        }
        // SAFETY: `flat` outlives the call and holds exactly 3·count
        // f64s; the shim validates n/index ranges again and returns
        // null on any failure (checked below).
        let handle = unsafe { ffi::appname_arap_mqwf_new(n_i32, flat.as_ptr(), count) };
        if handle.is_null() {
            return None;
        }
        Some(MqwfFactorization { handle, n })
    }

    /// System dimension.
    pub fn dim(&self) -> usize {
        self.n
    }

    /// Back-substitute one RHS. `None` on length mismatch or shim
    /// failure.
    pub fn solve(&self, b: &[f64]) -> Option<Vec<f64>> {
        if b.len() != self.n {
            return None;
        }
        let mut out = vec![0.0f64; self.n];
        // SAFETY: handle is live (owned by self); b and out are exactly
        // n elements, checked above; the shim writes at most n f64s.
        let rc = unsafe { ffi::appname_arap_mqwf_solve(self.handle, b.as_ptr(), out.as_mut_ptr()) };
        if rc != 0 {
            return None;
        }
        Some(out)
    }
}

/// Crate marker for the default (stub) build's smoke test.
pub const CRATE_NAME: &str = "appname-arap-sys";

#[cfg(test)]
mod tests {
    #[test]
    fn bootstrap_smoke() {
        assert_eq!(super::CRATE_NAME, "appname-arap-sys");
    }
}
