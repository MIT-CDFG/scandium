// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §18.4 "Puppet warp" + §45.5 "Puppet warp mesh and pins"
//! [NORMATIVE]; contract `contracts/commands/transform.puppet.yaml`; plan
//! §2.5 warps row — "puppet (MLS rigid + ARAP via libigl link-record)".
//!
//! Rights basis (V39 select-and-screen discipline, no release claim):
//! `contracts/algorithms/algorithms.yaml` area `warps_upright`, primary
//! `forward_warp_mls_arap_dlt` (level L2, basis [schaefer06, igarashi05,
//! sorkine07, hartley04]); ALGORITHMS.md §19.B — "Puppet warp: ARAP
//! triangle mesh with pins, exactly the SPEC §18.4 solver ([R-igarashi05]
//! [R-sorkine07], both L2; no Adobe puppet-warp patent surfaced in
//! targeted searches)"; "Point-handle warps: MLS deformation (rigid
//! variant default) [R-schaefer06] (L2)".
//!
//! Module map:
//! * [`mesh`] — §45.5 mesh generation (coverage → dilate → boundary →
//!   conforming Delaunay at the §18.4 edge budget; deterministic).
//! * [`arap`] — the §18.4 ARAP iteration driver (local rotation fit +
//!   §45.5 rigidity smoothing + rotation/depth pin semantics + warm
//!   start + the §45.5 f64 commit convergence rule) over the
//!   [`arap::ArapGlobalSolver`] boundary, plus the per-triangle
//!   inverse-affine rasterizer (§18.4 apply). THE ELECTED PRODUCTION
//!   GLOBAL-STEP BACKEND is libigl behind the O-INT-ruled cpp-shim
//!   consumption shape (deps/libigl.yaml + the 2026-08-17 ruling:
//!   file-level MPL via a shim TU from a pinned external checkout;
//!   riders: Eigen record, copyleft-exclusion guard, MLS = own) — see
//!   [`arap_libigl`]. The in-crate dense f64 factorization is a
//!   TEST-ONLY ORACLE (`#[cfg(test)]`) exercising the driver; it is not
//!   a shipping backend and does not supersede the election (the
//!   record's alternative clause stays dormant).
//! * [`arap_libigl`] — the feature-gated (`arap-libigl`) shim binding.
//!   Compiled only when the feature is enabled AND the pinned external
//!   checkouts are supplied (never vendored; record-first discipline).
//!
//! The MLS RIGID point-handle tier (the plan row's "MLS rigid" half) is
//! the OWN implementation in `appname-algo` (`appname_algo::mls`,
//! INTEGRATION-NOTES item 9, clean-room from schaefer06) — re-exported
//! here as the puppet session's interactive preview tier:
//! [`mls_rigid_grid`] evaluates the pins' displacement lattice at the
//! §45.4 field-representation shape for the workspace's live drag
//! preview, while the ARAP solve is the commit/convergence path (§45.5
//! preview-and-commit split).

pub mod arap;
#[cfg(feature = "arap-libigl")]
pub mod arap_libigl;
pub mod mesh;

use appname_algo::mls::{mls_displacement_grid, MlsGrid, MlsHandle, MlsVariant};

/// A puppet pin in SESSION form: the mesh point it grabbed at rest and
/// where it currently sits (drags mutate `target`). The wire form
/// (`transform.puppet` params) carries one `pos` per pin — see the
/// kind-arm docs for the recorded contract-gap escalation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PuppetPin {
    /// Rest position — where the pin was placed on the mesh (doc px).
    pub rest: [f64; 2],
    /// Current position — where the pin has been dragged (doc px).
    pub target: [f64; 2],
    /// §45.5 rotation pin: `None` = rotation free (ARAP finds it); set =
    /// triangles within 16 px take this angle as their fitted rotation.
    pub rotation_deg: Option<f64>,
    /// §45.5 overlap order (lexicographic max of (depth, triangleIndex)).
    pub depth: i64,
}

impl PuppetPin {
    /// A pin at rest (the wire form's expressible state).
    pub fn at(pos: [f64; 2]) -> PuppetPin {
        PuppetPin {
            rest: pos,
            target: pos,
            rotation_deg: None,
            depth: 0,
        }
    }
}

/// §45.5 rotation-pin influence radius (px).
pub const ROTATION_PIN_RADIUS: f64 = 16.0;

/// The MLS RIGID interactive tier (plan §2.5 "MLS rigid" — own
/// implementation, `appname-algo` item 9): evaluate the point-handle
/// displacement lattice for the current pin set at `spacing` px over a
/// `w × h` px domain (the §45.4 field-representation shape the preview
/// remap consumes). Handles are the pins' rest → target pairs; the
/// rigid variant is the §19.B default.
pub fn mls_rigid_grid(pins: &[PuppetPin], w: usize, h: usize, spacing: usize) -> MlsGrid {
    let handles: Vec<MlsHandle> = pins
        .iter()
        .map(|p| {
            (
                [p.rest[0] as f32, p.rest[1] as f32],
                [p.target[0] as f32, p.target[1] as f32],
            )
        })
        .collect();
    mls_displacement_grid(w, h, spacing, &handles, MlsVariant::Rigid, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mls_rigid_tier_rest_pins_are_identity() {
        let pins = vec![PuppetPin::at([10.0, 10.0]), PuppetPin::at([40.0, 30.0])];
        let g = mls_rigid_grid(&pins, 64, 64, 8);
        for (dx, dy) in g.dx.iter().zip(g.dy.iter()) {
            assert_eq!((*dx, *dy), (0.0, 0.0), "rest handles ⇒ u ≡ 0");
        }
    }

    #[test]
    fn mls_rigid_tier_uniform_translation() {
        // Every pin dragged by the same delta ⇒ the rigid deformation is
        // that translation everywhere (paper §2.3 translation case).
        let mut pins = vec![PuppetPin::at([10.0, 10.0]), PuppetPin::at([40.0, 30.0])];
        for p in &mut pins {
            p.target = [p.rest[0] + 5.0, p.rest[1] - 3.0];
        }
        let g = mls_rigid_grid(&pins, 64, 64, 16);
        let d = g.sample(24.0, 24.0);
        assert!(
            (d[0] - 5.0).abs() < 1e-4 && (d[1] + 3.0).abs() < 1e-4,
            "{d:?}"
        );
    }
}

#[cfg(test)]
mod gpl_subtree_exclusion_guard {
    /// deps/libigl.yaml rider (2) — the copyleft-exclusion guard,
    /// grep-proven: no CODE line anywhere in this crate's puppet module,
    /// the shim TU, or the build script references libigl's `copyleft/`
    /// subtree. Comment lines legitimately NAME the guard (this module,
    /// the record citations); what compiles is what the guard binds.
    #[test]
    fn no_code_reference_into_the_gpl_subtree() {
        // The scanned pattern, assembled so the guard's own code lines
        // never contain it verbatim.
        let pat = String::from("copy") + "left";
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let sys = base.parent().unwrap().join("appname-arap-sys");
        let files = [
            base.join("src/puppet/mod.rs"),
            base.join("src/puppet/mesh.rs"),
            base.join("src/puppet/arap.rs"),
            base.join("src/puppet/arap_libigl.rs"),
            sys.join("src/lib.rs"),
            sys.join("shim/arap_global_shim.cpp"),
            sys.join("build.rs"),
        ];
        for f in files {
            let text = std::fs::read_to_string(&f)
                .unwrap_or_else(|e| panic!("guard target {f:?} unreadable: {e}"));
            for (ln, line) in text.lines().enumerate() {
                let t = line.trim_start();
                let is_comment = t.starts_with("//")
                    || t.starts_with("/*")
                    || t.starts_with("*")
                    || t.starts_with("#")   // shell-style / attribute-free lines never carry includes here
                        && !t.starts_with("#include");
                if !is_comment && t.to_ascii_lowercase().contains(&pat) {
                    panic!(
                        "{pat} reference on a CODE line ({}:{}): {line}",
                        f.display(),
                        ln + 1
                    );
                }
            }
        }
    }
}
