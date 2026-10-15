// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §15.4 (dynamics model: inputs, outputs, combination rule),
//! §15.7 (brush presets — the `.bbrush` v1 field set and defaults),
//! §10.3 stroke commons (`sizePx` 0.1..=10 000 as the diameter range).
//! DD ⑮. Semantics informed by `MEMO-libmypaint-brush-dynamics` (dab/
//! dynamics parameter concepts only; Appname's multiplicative
//! monotone-cubic model is the SPEC's own design and nothing here is ported
//! from libmypaint source).
//!
//! CP-0.3 v1 scope (plan §2.2 "Brush engine v1"): the engine-input subset of
//! the §15.7 preset — round tip, diameter/spacing/hardness/roundness/angle,
//! scatter parameters, and the six-output dynamics table. Deliberately NOT
//! in this struct yet (arrive with their features): `tip: Image/Dual`
//! (textured tips), `wet_edges`/`wet_strength`, `buildup`/`airbrush_rate`,
//! `mode` (the entry point — brush vs eraser command kind — selects the
//! presentation path in v1), `smudge`, `texture_lock`, and the `.bbrush`
//! JSON (de)serialization (§28 resource index, CP-0.5) and `.myb` importer
//! (CP-0.7, O-ALGO).

use std::fmt;

use crate::brush::curve::{CurveError, MonotoneCurve};

/// §15.4 `DynInput` — which normalized input drives a curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DynInput {
    /// Sample pressure, already `0..=1` (§15.1).
    Pressure,
    /// Tilt x, normalized `(x+1)/2`.
    TiltX,
    /// Tilt y, normalized `(y+1)/2`.
    TiltY,
    /// `min(1, v / 3000 px·s⁻¹)`.
    Velocity,
    /// Barrel rotation, `(θ mod 2π)/2π`.
    Rotation,
    /// One uniform draw per dab from the stroke's seeded PCG32 (§15.4).
    Random,
    /// Stroke heading, `(atan2 mod 2π)/2π`.
    Direction,
}

/// §15.4 `DynOutput` — the ordinal IS the PCG32 sequence id of the output's
/// jitter/scatter sub-stream (determinism-frozen; see `brush` module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DynOutput {
    Size = 0,
    Opacity = 1,
    Flow = 2,
    Angle = 3,
    Scatter = 4,
    Roundness = 5,
}

impl DynOutput {
    pub const ALL: [DynOutput; 6] = [
        DynOutput::Size,
        DynOutput::Opacity,
        DynOutput::Flow,
        DynOutput::Angle,
        DynOutput::Scatter,
        DynOutput::Roundness,
    ];
}

/// One §15.4 response curve: input selector, monotone-cubic control points
/// over `[0,1] → [0,1]`, and a jitter amplitude.
#[derive(Clone, Debug, PartialEq)]
pub struct DynamicCurve {
    pub input: DynInput,
    /// 2–16 control points, `x` strictly increasing, all coords in `[0,1]`.
    pub points: Vec<[f32; 2]>,
    /// `0..=1`; §15.4: `value ← clamp(value + U(−1,1)·jitter·range)`.
    pub jitter: f32,
}

impl DynamicCurve {
    /// A linear pressure-response curve with no jitter — the `.bbrush`
    /// default entry (§15.7).
    pub fn pressure_linear() -> DynamicCurve {
        DynamicCurve {
            input: DynInput::Pressure,
            points: vec![[0.0, 0.0], [1.0, 1.0]],
            jitter: 0.0,
        }
    }
}

/// §15.4 dynamics table: 0..n curves per output (the §15.7 `.bbrush` array
/// form). An empty list = constant multiplier 1 (no offset for angle).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrushDynamics {
    pub size: Vec<DynamicCurve>,
    pub opacity: Vec<DynamicCurve>,
    pub flow: Vec<DynamicCurve>,
    pub angle: Vec<DynamicCurve>,
    pub scatter: Vec<DynamicCurve>,
    pub roundness: Vec<DynamicCurve>,
}

impl BrushDynamics {
    pub fn curves(&self, output: DynOutput) -> &[DynamicCurve] {
        match output {
            DynOutput::Size => &self.size,
            DynOutput::Opacity => &self.opacity,
            DynOutput::Flow => &self.flow,
            DynOutput::Angle => &self.angle,
            DynOutput::Scatter => &self.scatter,
            DynOutput::Roundness => &self.roundness,
        }
    }
}

/// §15.7 `ScatterParams`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScatterParams {
    /// Offset span in tip diameters; effective span = `amount ·
    /// scatter_mult` (§15.4), offset = `U(−1,1) · span · diameter` (§15.7).
    pub amount: f32,
    /// Add an independent tangent-axis draw.
    pub both_axes: bool,
    /// Dabs spawned per march position, `1..=16`.
    pub count: u8,
    /// `0..=1`: each spawn reduces to `round(count·(1 − U(0,1)·countJitter))`,
    /// minimum 1 (§15.7).
    pub count_jitter: f32,
}

impl Default for ScatterParams {
    /// `.bbrush` defaults (§15.7).
    fn default() -> ScatterParams {
        ScatterParams {
            amount: 0.0,
            both_axes: false,
            count: 1,
            count_jitter: 0.0,
        }
    }
}

/// The v1 engine-input brush preset (§15.7 subset — module docs list what is
/// deferred and why). Round analytic tip only in v1.
#[derive(Clone, Debug, PartialEq)]
pub struct BrushPreset {
    /// Tip diameter in document px, `0.1..=10 000` (§10.3 `sizePx` range);
    /// the command's `sizePx` overrides it per stroke.
    pub diameter: f32,
    /// Stamp spacing as a fraction of tip diameter (§15.3); `.bbrush`
    /// default 0.1. v1 pin: `(0, 10]` (the §15.3 Δs clamp bounds tiny
    /// values; large values are legal sparse dabbing).
    pub spacing: f32,
    /// `0..=1`: 1 = hard disk, 0 = soft falloff (§15.5 `tip_coverage`).
    pub hardness: f32,
    /// `0.05..=1` (§15.4 clamp span): minor-axis squash of the tip.
    pub roundness: f32,
    /// Base tip angle in radians (serialized form is degrees, §0.3 — the
    /// `.bbrush` layer converts; the engine consumes radians).
    pub angle_rad: f32,
    /// §15.7 scatter block.
    pub scatter: ScatterParams,
    /// §15.4 dynamics table.
    pub dynamics: BrushDynamics,
}

impl BrushPreset {
    /// The `.bbrush` default preset (§15.7 "Soft Round Pressure" defaults):
    /// diameter 64, spacing 0.1, hardness 0, roundness 1, angle 0, pressure→
    /// size and pressure→flow linear curves, no scatter.
    pub fn soft_round_pressure() -> BrushPreset {
        BrushPreset {
            diameter: 64.0,
            spacing: 0.1,
            hardness: 0.0,
            roundness: 1.0,
            angle_rad: 0.0,
            scatter: ScatterParams::default(),
            dynamics: BrushDynamics {
                size: vec![DynamicCurve::pressure_linear()],
                flow: vec![DynamicCurve::pressure_linear()],
                ..BrushDynamics::default()
            },
        }
    }

    /// A hard round constant preset (no dynamics) — handy oracle shape.
    pub fn hard_round(diameter: f32) -> BrushPreset {
        BrushPreset {
            diameter,
            spacing: 0.1,
            hardness: 1.0,
            roundness: 1.0,
            angle_rad: 0.0,
            scatter: ScatterParams::default(),
            dynamics: BrushDynamics::default(),
        }
    }

    /// Validate every field against its documented range and build-check
    /// every curve. The stroke planner calls this before any math.
    pub fn validate(&self) -> Result<(), PresetError> {
        if !self.diameter.is_finite() || !(0.1..=10_000.0).contains(&self.diameter) {
            return Err(PresetError::DiameterOutOfRange(self.diameter));
        }
        if !self.spacing.is_finite() || self.spacing <= 0.0 || self.spacing > 10.0 {
            return Err(PresetError::SpacingOutOfRange(self.spacing));
        }
        if !self.hardness.is_finite() || !(0.0..=1.0).contains(&self.hardness) {
            return Err(PresetError::HardnessOutOfRange(self.hardness));
        }
        if !self.roundness.is_finite() || !(0.05..=1.0).contains(&self.roundness) {
            return Err(PresetError::RoundnessOutOfRange(self.roundness));
        }
        if !self.angle_rad.is_finite() {
            return Err(PresetError::AngleNotFinite);
        }
        let s = &self.scatter;
        if !s.amount.is_finite() || !(0.0..=100.0).contains(&s.amount) {
            return Err(PresetError::ScatterAmountOutOfRange(s.amount));
        }
        if !(1..=16).contains(&s.count) {
            return Err(PresetError::ScatterCountOutOfRange(s.count));
        }
        if !s.count_jitter.is_finite() || !(0.0..=1.0).contains(&s.count_jitter) {
            return Err(PresetError::CountJitterOutOfRange(s.count_jitter));
        }
        for output in DynOutput::ALL {
            for (index, c) in self.dynamics.curves(output).iter().enumerate() {
                if !c.jitter.is_finite() || !(0.0..=1.0).contains(&c.jitter) {
                    return Err(PresetError::JitterOutOfRange { output, index });
                }
                MonotoneCurve::build(&c.points).map_err(|err| PresetError::Curve {
                    output,
                    index,
                    err,
                })?;
            }
        }
        Ok(())
    }
}

/// Preset validation failures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PresetError {
    DiameterOutOfRange(f32),
    SpacingOutOfRange(f32),
    HardnessOutOfRange(f32),
    RoundnessOutOfRange(f32),
    AngleNotFinite,
    ScatterAmountOutOfRange(f32),
    ScatterCountOutOfRange(u8),
    CountJitterOutOfRange(f32),
    JitterOutOfRange {
        output: DynOutput,
        index: usize,
    },
    Curve {
        output: DynOutput,
        index: usize,
        err: CurveError,
    },
}

impl fmt::Display for PresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PresetError::DiameterOutOfRange(v) => {
                write!(f, "diameter {v} outside 0.1..=10000 (§10.3)")
            }
            PresetError::SpacingOutOfRange(v) => write!(f, "spacing {v} outside (0, 10]"),
            PresetError::HardnessOutOfRange(v) => write!(f, "hardness {v} outside [0, 1]"),
            PresetError::RoundnessOutOfRange(v) => {
                write!(f, "roundness {v} outside [0.05, 1] (§15.4)")
            }
            PresetError::AngleNotFinite => write!(f, "angle not finite"),
            PresetError::ScatterAmountOutOfRange(v) => {
                write!(f, "scatter amount {v} outside [0, 100]")
            }
            PresetError::ScatterCountOutOfRange(v) => {
                write!(f, "scatter count {v} outside 1..=16 (§15.7)")
            }
            PresetError::CountJitterOutOfRange(v) => {
                write!(f, "scatter countJitter {v} outside [0, 1]")
            }
            PresetError::JitterOutOfRange { output, index } => {
                write!(f, "curve {index} on {output:?}: jitter outside [0, 1]")
            }
            PresetError::Curve { output, index, err } => {
                write!(f, "curve {index} on {output:?}: {err}")
            }
        }
    }
}

impl std::error::Error for PresetError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_presets_validate() {
        BrushPreset::soft_round_pressure().validate().unwrap();
        BrushPreset::hard_round(10.0).validate().unwrap();
        BrushPreset::hard_round(0.1).validate().unwrap();
        BrushPreset::hard_round(10_000.0).validate().unwrap();
    }

    #[test]
    fn field_ranges_reject() {
        let mut p = BrushPreset::soft_round_pressure();
        p.diameter = 0.05;
        assert_eq!(p.validate(), Err(PresetError::DiameterOutOfRange(0.05)));
        let mut p = BrushPreset::soft_round_pressure();
        p.diameter = 10_001.0;
        assert!(matches!(
            p.validate(),
            Err(PresetError::DiameterOutOfRange(_))
        ));
        let mut p = BrushPreset::soft_round_pressure();
        p.spacing = 0.0;
        assert!(matches!(
            p.validate(),
            Err(PresetError::SpacingOutOfRange(_))
        ));
        let mut p = BrushPreset::soft_round_pressure();
        p.hardness = 1.5;
        assert!(matches!(
            p.validate(),
            Err(PresetError::HardnessOutOfRange(_))
        ));
        let mut p = BrushPreset::soft_round_pressure();
        p.roundness = 0.01;
        assert!(matches!(
            p.validate(),
            Err(PresetError::RoundnessOutOfRange(_))
        ));
        let mut p = BrushPreset::soft_round_pressure();
        p.angle_rad = f32::NAN;
        assert_eq!(p.validate(), Err(PresetError::AngleNotFinite));
        let mut p = BrushPreset::soft_round_pressure();
        p.scatter.count = 0;
        assert_eq!(p.validate(), Err(PresetError::ScatterCountOutOfRange(0)));
        let mut p = BrushPreset::soft_round_pressure();
        p.scatter.count = 17;
        assert_eq!(p.validate(), Err(PresetError::ScatterCountOutOfRange(17)));
    }

    #[test]
    fn curve_errors_carry_their_output_slot() {
        let mut p = BrushPreset::soft_round_pressure();
        p.dynamics.opacity = vec![DynamicCurve {
            input: DynInput::Pressure,
            points: vec![[0.0, 0.0]],
            jitter: 0.0,
        }];
        assert_eq!(
            p.validate(),
            Err(PresetError::Curve {
                output: DynOutput::Opacity,
                index: 0,
                err: CurveError::TooFewPoints
            })
        );
        let mut p = BrushPreset::soft_round_pressure();
        p.dynamics.size[0].jitter = 2.0;
        assert_eq!(
            p.validate(),
            Err(PresetError::JitterOutOfRange {
                output: DynOutput::Size,
                index: 0
            })
        );
    }

    #[test]
    fn output_ordinals_are_the_frozen_stream_ids() {
        // §15.4: PCG sequence id = output ordinal. Changing these would
        // silently re-seed every stroke — freeze them.
        assert_eq!(DynOutput::Size as u8, 0);
        assert_eq!(DynOutput::Opacity as u8, 1);
        assert_eq!(DynOutput::Flow as u8, 2);
        assert_eq!(DynOutput::Angle as u8, 3);
        assert_eq!(DynOutput::Scatter as u8, 4);
        assert_eq!(DynOutput::Roundness as u8, 5);
    }
}
