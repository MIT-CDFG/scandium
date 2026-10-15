// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §15.4 — a `DynamicCurve` is a **monotone cubic Hermite**
//! interpolant over `[0,1] → [0,1]` through 2–16 control points, with
//! tangents limited by the Fritsch–Carlson method "[LEGAL: Fritsch & Carlson
//! 1980, public numerical-analysis literature]" so the curve never
//! overshoots or oscillates between points.
//!
//! v1 pins (documented; determinism-frozen):
//! * Control-point `x` must be strictly increasing within `[0, 1]`; `y`
//!   within `[0, 1]`. Endpoints need not sit at x = 0 / x = 1.
//! * Inputs outside the knot span evaluate as **constant extension** (the
//!   nearest endpoint's value).
//! * Evaluation clamps its result to `[0, 1]` (Fritsch–Carlson already
//!   bounds it by the knot values; the clamp is a defensive invariant).
//! * All arithmetic is plain f32 +,−,×,÷ and `sqrt` — correctly-rounded IEEE
//!   ops, so evaluation is bit-identical across platforms (§34.4).

use std::fmt;

/// Why a control-point list is not a valid §15.4 curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveError {
    /// Fewer than 2 points.
    TooFewPoints,
    /// More than 16 points.
    TooManyPoints,
    /// A coordinate is NaN or infinite.
    NonFinite { index: usize },
    /// An `x` or `y` coordinate is outside `[0, 1]`.
    OutOfRange { index: usize },
    /// `x` values are not strictly increasing.
    XNotIncreasing { index: usize },
}

impl fmt::Display for CurveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CurveError::TooFewPoints => write!(f, "curve needs at least 2 control points"),
            CurveError::TooManyPoints => write!(f, "curve allows at most 16 control points"),
            CurveError::NonFinite { index } => write!(f, "control point {index} not finite"),
            CurveError::OutOfRange { index } => write!(f, "control point {index} outside [0,1]"),
            CurveError::XNotIncreasing { index } => {
                write!(f, "control point {index}: x not strictly increasing")
            }
        }
    }
}

impl std::error::Error for CurveError {}

/// A built (validated, tangent-limited) monotone cubic Hermite curve.
#[derive(Clone, Debug, PartialEq)]
pub struct MonotoneCurve {
    xs: Vec<f32>,
    ys: Vec<f32>,
    /// Fritsch–Carlson-limited tangents, one per knot.
    ms: Vec<f32>,
}

impl MonotoneCurve {
    /// Validate control points and precompute Fritsch–Carlson tangents.
    pub fn build(points: &[[f32; 2]]) -> Result<MonotoneCurve, CurveError> {
        if points.len() < 2 {
            return Err(CurveError::TooFewPoints);
        }
        if points.len() > 16 {
            return Err(CurveError::TooManyPoints);
        }
        for (i, p) in points.iter().enumerate() {
            if !p[0].is_finite() || !p[1].is_finite() {
                return Err(CurveError::NonFinite { index: i });
            }
            if !(0.0..=1.0).contains(&p[0]) || !(0.0..=1.0).contains(&p[1]) {
                return Err(CurveError::OutOfRange { index: i });
            }
        }
        for i in 1..points.len() {
            if points[i][0] <= points[i - 1][0] {
                return Err(CurveError::XNotIncreasing { index: i });
            }
        }
        let xs: Vec<f32> = points.iter().map(|p| p[0]).collect();
        let ys: Vec<f32> = points.iter().map(|p| p[1]).collect();
        let n = xs.len();
        // Secant slopes per interval.
        let deltas: Vec<f32> = (0..n - 1)
            .map(|i| (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]))
            .collect();
        // Initial tangents: one-sided at the ends, secant average interior,
        // zeroed at local extrema (sign change or flat neighbor).
        let mut ms = vec![0.0f32; n];
        ms[0] = deltas[0];
        ms[n - 1] = deltas[n - 2];
        for i in 1..n - 1 {
            if deltas[i - 1] * deltas[i] <= 0.0 {
                ms[i] = 0.0;
            } else {
                ms[i] = 0.5 * (deltas[i - 1] + deltas[i]);
            }
        }
        // Fritsch–Carlson circle limit per interval: α² + β² ≤ 9.
        for i in 0..n - 1 {
            let d = deltas[i];
            if d == 0.0 {
                ms[i] = 0.0;
                ms[i + 1] = 0.0;
                continue;
            }
            let a = ms[i] / d;
            let b = ms[i + 1] / d;
            let s = a * a + b * b;
            if s > 9.0 {
                let tau = 3.0 / s.sqrt();
                ms[i] = tau * a * d;
                ms[i + 1] = tau * b * d;
            }
        }
        Ok(MonotoneCurve { xs, ys, ms })
    }

    /// Evaluate at `x` (clamped to the knot span; result clamped to [0,1]).
    pub fn eval(&self, x: f32) -> f32 {
        let n = self.xs.len();
        let x = if x.is_nan() { 0.0 } else { x };
        if x <= self.xs[0] {
            return self.ys[0];
        }
        if x >= self.xs[n - 1] {
            return self.ys[n - 1];
        }
        // ≤ 16 knots: linear scan is deterministic and fast.
        let mut i = 0;
        while i + 2 < n && x >= self.xs[i + 1] {
            i += 1;
        }
        let h = self.xs[i + 1] - self.xs[i];
        let u = (x - self.xs[i]) / h;
        let u2 = u * u;
        let u3 = u2 * u;
        let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
        let h10 = u3 - 2.0 * u2 + u;
        let h01 = -2.0 * u3 + 3.0 * u2;
        let h11 = u3 - u2;
        let y = self.ys[i] * h00
            + self.ms[i] * h * h10
            + self.ys[i + 1] * h01
            + self.ms[i + 1] * h * h11;
        y.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_rejects_bad_point_lists() {
        assert_eq!(
            MonotoneCurve::build(&[[0.0, 0.0]]),
            Err(CurveError::TooFewPoints)
        );
        let many: Vec<[f32; 2]> = (0..17).map(|i| [i as f32 / 16.0, 0.5]).collect();
        assert_eq!(MonotoneCurve::build(&many), Err(CurveError::TooManyPoints));
        assert_eq!(
            MonotoneCurve::build(&[[0.0, 0.0], [f32::NAN, 1.0]]),
            Err(CurveError::NonFinite { index: 1 })
        );
        assert_eq!(
            MonotoneCurve::build(&[[0.0, 0.0], [1.1, 1.0]]),
            Err(CurveError::OutOfRange { index: 1 })
        );
        assert_eq!(
            MonotoneCurve::build(&[[0.0, 0.0], [0.5, 0.5], [0.5, 1.0]]),
            Err(CurveError::XNotIncreasing { index: 2 })
        );
    }

    #[test]
    fn interpolates_knots_exactly() {
        let pts = [[0.0, 0.0], [0.25, 0.21], [0.75, 0.83], [1.0, 1.0]];
        let c = MonotoneCurve::build(&pts).unwrap();
        for p in pts {
            assert_eq!(c.eval(p[0]), p[1], "knot ({}, {})", p[0], p[1]);
        }
    }

    #[test]
    fn identity_two_point_curve_is_linear() {
        let c = MonotoneCurve::build(&[[0.0, 0.0], [1.0, 1.0]]).unwrap();
        for i in 0..=100 {
            let x = i as f32 / 100.0;
            assert!((c.eval(x) - x).abs() < 1e-6, "x={x}");
        }
    }

    #[test]
    fn monotone_data_evaluates_monotone_no_overshoot() {
        // A step-like data set that a natural cubic spline would overshoot.
        let c = MonotoneCurve::build(&[[0.0, 0.0], [0.4, 0.02], [0.6, 0.98], [1.0, 1.0]]).unwrap();
        let mut prev = -1.0f32;
        for i in 0..=1000 {
            let x = i as f32 / 1000.0;
            let y = c.eval(x);
            assert!((0.0..=1.0).contains(&y), "bounded at x={x}");
            assert!(y >= prev - 1e-6, "monotone at x={x}: {y} < {prev}");
            prev = y;
        }
    }

    #[test]
    fn flat_segments_stay_flat() {
        let c = MonotoneCurve::build(&[[0.0, 0.5], [0.5, 0.5], [1.0, 1.0]]).unwrap();
        for i in 0..=50 {
            let x = 0.5 * i as f32 / 50.0;
            assert_eq!(c.eval(x), 0.5, "flat interval must not wiggle at x={x}");
        }
    }

    #[test]
    fn constant_extension_outside_knot_span() {
        let c = MonotoneCurve::build(&[[0.2, 0.3], [0.8, 0.9]]).unwrap();
        assert_eq!(c.eval(0.0), 0.3);
        assert_eq!(c.eval(-5.0), 0.3);
        assert_eq!(c.eval(1.0), 0.9);
        assert_eq!(c.eval(7.0), 0.9);
    }

    #[test]
    fn non_monotone_data_is_bounded_by_knot_values() {
        // A peak: up then down. Fritsch–Carlson zeroes the peak tangent.
        let c = MonotoneCurve::build(&[[0.0, 0.0], [0.5, 1.0], [1.0, 0.0]]).unwrap();
        for i in 0..=1000 {
            let x = i as f32 / 1000.0;
            let y = c.eval(x);
            assert!((0.0..=1.0).contains(&y), "x={x} y={y}");
        }
        assert_eq!(c.eval(0.5), 1.0);
    }
}
