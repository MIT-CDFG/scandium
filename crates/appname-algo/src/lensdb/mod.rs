// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Lens profile database reader (own Rust) — CP-1.0 Row C.
//!
//! Spec: ALGORITHMS.md §21 (lens correction & geometric camera models —
//! "Profile database: lensfun — consumed as **data only**"); dossier
//! `spec/algorithms/dossiers/upright-lens.yaml`; ES Part 43 §43.3 (RAW
//! develop pipeline, reserved sub-stage 4b `lensCorrection`, Post-1.0)
//! with §8.3's stage list; contract row `lens_correction`:
//! `primary: brown_conrady_plus_division_model (L0)`, `database:
//! {name: lensfun_db_own_rust_reader, data_license: CC-BY-SA-3.0,
//! library_linked: false}`.
//!
//! CLEAN-ROOM / LICENSE BOUNDARY (rights-register lensfun-db row;
//! INTEGRATION-NOTES §1 row 7 + §5.6): the lensfun LIBRARY is LGPL-3.0
//! and is NOT linked, NOT vendored, NOT consulted — no lens-correction
//! library code was read for this module. The vendored acquisition
//! (`appname-proto/vendor/lensfun-db/`, upstream pin `6804b5f5…`) is
//! the CC-BY-SA-3.0 DATABASE only; this reader is own Rust written
//! from `lensfun-database.dtd` / `lensfun-database.xsd` and the XML
//! files' observable structure. CC-BY-SA-3.0 is not in the policy
//! `weights_allowed` list — the acquisition rests on the contract's
//! explicit `lens_correction.database` decision. Obligations (surface
//! with the data package, O-BUILD staging): attribution in credits +
//! THIRD-PARTY-NOTICES; share-alike = database modifications
//! contributed back upstream; user-replaceable data dir.
//!
//! Submodules:
//! - [`xml`] — minimal own XML subset parser (deterministic, iterative,
//!   typed errors with positions; scope fixed by the corpus survey);
//! - [`model`] — profile model exactly as the database records it
//!   (mounts, cameras, lenses; `ptlens`/`poly3`/`poly5` distortion,
//!   `linear`/`poly3` TCA, `pa` vignetting, with focal/aperture/
//!   distance breakpoints);
//! - [`reader`] — data-directory reader (sorted file order, duplicate
//!   identities surfaced as warnings, canonical merged database);
//! - [`lookup`] — deterministic camera/lens matching with total
//!   tie-breaks + per-shot calibration resolution (linear parameter
//!   interpolation between focal breakpoints, exact at breakpoints,
//!   clamped outside) and the documented model mapping onto the
//!   engine's Brown–Conrady corrector (\[brown66\]; the division model
//!   \[fitzgibbon01\] is the corrector's second form, no DB source).
//!
//! ADAPTER SEAM (brokered): O-IO's RAW develop `lens_correction` stage
//! consumes [`reader::LensDatabase`] + [`lookup::resolve_calibration`]
//! through the `appname.command.v1` envelope spine with captured
//! parameters (D-48); this module never touches documents or the
//! filesystem outside the data dir handed to [`reader::read_dir`].
//! Staging the CC-BY-SA-3.0 `db/` payload into the distribution's data
//! dir is O-BUILD packaging territory, not this module's.

pub mod lookup;
pub mod model;
pub mod reader;
pub mod xml;

pub use lookup::{
    normalize_name, resolve_calibration, BrownConradyRadial, LookupError, ResolvedCalibration,
    ResolvedVignetting,
};
pub use model::{
    AspectRatio, Calibration, Camera, DistortionCal, DistortionModel, Lens, LensType, LocalName,
    Mount, RangeOrValue, TcaCal, TcaModel, VignettingCal,
};
pub use reader::{from_named_documents, read_dir, DbError, LensDatabase};
pub use xml::{XmlError, XmlErrorKind};
