// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §16.1 — "The registry makes filters discoverable to the UI,
//! scripting, and smart filters. Adding a filter requires no core edits."
//! Deterministic iteration (sorted ids) for stable enumeration surfaces.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::traits::Filter;

/// Filter execution/validation failures (mapped onto the wire taxonomy by
//  the command seam when it lands).
#[derive(Clone, Debug, PartialEq)]
pub enum FilterError {
    /// Unknown filter id (→ stage-4-analog reject).
    UnknownFilter(String),
    /// A param failed its §16.3 range/cross-field row.
    ParamOutOfRange { param: &'static str, detail: String },
    /// A required param is missing.
    ParamMissing(&'static str),
    /// The provided input plane does not cover the declared ROI (evaluator
    /// contract violation — a programming error surfaced as a typed error,
    /// never a panic).
    RoiNotCovered,
}

impl core::fmt::Display for FilterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FilterError::UnknownFilter(id) => write!(f, "filter id {id:?} is not registered"),
            FilterError::ParamOutOfRange { param, detail } => {
                write!(f, "param {param:?}: {detail}")
            }
            FilterError::ParamMissing(p) => write!(f, "param {p:?} is required"),
            FilterError::RoiNotCovered => {
                write!(
                    f,
                    "input plane does not cover the filter's declared ROI (DD ⑯)"
                )
            }
        }
    }
}

impl std::error::Error for FilterError {}

/// §16.1 `FilterRegistry`.
#[derive(Default, Clone)]
pub struct FilterRegistry {
    by_id: BTreeMap<&'static str, Arc<dyn Filter>>,
}

impl FilterRegistry {
    pub fn new() -> FilterRegistry {
        FilterRegistry::default()
    }

    /// Register one filter. Panics on a duplicate id — construction is
    /// programmer-controlled (the assembly precedent).
    pub fn with(mut self, filter: Arc<dyn Filter>) -> FilterRegistry {
        let id = filter.id();
        assert!(
            id.starts_with("filter.")
                && id[7..].chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "FilterIds are filter.snake_case (§16.1): {id}"
        );
        assert!(!self.by_id.contains_key(id), "filter {id} registered twice");
        self.by_id.insert(id, filter);
        self
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn Filter>> {
        self.by_id.get(id)
    }

    /// Sorted, deterministic id listing (enumeration surfaces).
    pub fn ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.by_id.keys().copied()
    }

    /// The registered filters in id order (the §12.1 OpImpl bridge walks
    /// this to register the corpus construction-only — `op_bridge`).
    pub fn filters(&self) -> impl Iterator<Item = &Arc<dyn Filter>> + '_ {
        self.by_id.values()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}
