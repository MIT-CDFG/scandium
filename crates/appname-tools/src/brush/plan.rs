// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §15.3 (arc-length march, first/last-dab rules, dynamics
//! interpolation), §15.4 (response-curve evaluation, combination rule,
//! jitter, PCG32 sub-streams), §15.7 (scatter spawning); §10.3 stroke
//! commons (`sizePx`/`opacity`/`flow` params); contracts
//! `brush.stroke.yaml`/`eraser.stroke.yaml` (`seed` is the command-recorded
//! per-stroke dynamics seed, §15.4 — replay is bit-identical, §34.4).
//!
//! This stage is the **deterministic dab sequence** producer the plan row
//! demands: `plan_stroke(samples, preset, params) → StrokePlan`. It is pure
//! (no buffers, no I/O); the RNG draw schedule is frozen in the `brush`
//! module docs.

use appname_core::IRect;

use crate::brush::curve::MonotoneCurve;
use crate::brush::preset::{BrushPreset, DynInput, DynOutput, PresetError};
use crate::brush::rng::Pcg32;
use crate::brush::sample::{validate_samples, InputSample, SampleBlobError};

/// §15.4: the `DynInput::Random` value draws from its own dedicated PCG32
/// sequence (v1 pin — keeps every output sub-stream independent of preset
/// curve layout). Ordinals 0–5 are the `DynOutput` streams.
const RANDOM_INPUT_STREAM: u64 = 6;

/// Velocity normalization constant (§15.4): `min(1, v / 3000 px·s⁻¹)`.
const VELOCITY_NORM: f32 = 3000.0;

/// Stroke-level parameters, 1:1 with the §10.3 stroke commons.
#[derive(Clone, Copy, Debug)]
pub struct StrokeParams {
    /// `sizePx`: overrides the preset diameter (0.1..=10 000, §10.3).
    pub size_px: Option<f32>,
    /// `hardness`: overrides the preset tip hardness (0..=1, §15.7);
    /// absent = preset hardness. Symmetric with `size_px` (§10.3 override
    /// idiom); the value flows straight to `Dab.hardness` (§15.5
    /// `tip_coverage`), the same field the preset otherwise supplies.
    pub hardness: Option<f32>,
    /// `opacity` (0..=1, default 1): the §15.5 per-stroke presentation
    /// ceiling.
    pub opacity: f32,
    /// `flow` (0..=1, default 1): per-dab accumulation base.
    pub flow: f32,
    /// The command-recorded PCG32 seed (§15.4).
    pub seed: u64,
}

impl Default for StrokeParams {
    fn default() -> StrokeParams {
        StrokeParams {
            size_px: None,
            hardness: None,
            opacity: 1.0,
            flow: 1.0,
            seed: 0,
        }
    }
}

/// One planned dab (§15.3 `Stamp` shape; scatter offsets are baked into
/// `center`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dab {
    /// Document-space center, sub-pixel.
    pub center: [f64; 2],
    /// Evaluated tip diameter, px (§15.4 size output applied).
    pub diameter: f32,
    /// Evaluated tip angle, radians.
    pub angle_rad: f32,
    /// Evaluated roundness, `0.05..=1`.
    pub roundness: f32,
    /// Tip hardness (§15.5 `tip_coverage`): the command `hardness` override
    /// when present, else the preset's value. Stroke-constant in v1 — not a
    /// §15.4 output.
    pub hardness: f32,
    /// Evaluated per-dab flow (`params.flow · flow_mult`, §15.4/§15.5).
    pub flow: f32,
    /// Evaluated per-dab opacity (`params.opacity · opacity_mult`).
    /// Recorded for tests/forward-compat; v1 presentation applies the
    /// stroke ceiling (`params.opacity`) per §15.5 step 4 (module pin).
    pub opacity: f32,
}

impl Dab {
    /// Integer pixel rect of every pixel center this dab can touch,
    /// including the 1 px analytic-AA margin (§15.5 tile-touch rule).
    pub fn pixel_rect(&self) -> IRect {
        let half = (self.diameter as f64) * 0.5 + 1.0;
        let min_x = (self.center[0] - half - 0.5).ceil();
        let min_y = (self.center[1] - half - 0.5).ceil();
        let max_x = (self.center[0] + half - 0.5).floor() + 1.0;
        let max_y = (self.center[1] + half - 0.5).floor() + 1.0;
        IRect::new(
            clamp_i32(min_x),
            clamp_i32(min_y),
            clamp_i32(max_x),
            clamp_i32(max_y),
        )
    }
}

fn clamp_i32(v: f64) -> i32 {
    v.clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

/// The deterministic dab sequence for one stroke.
#[derive(Clone, Debug)]
pub struct StrokePlan {
    /// Dabs in march order (§15.5 accumulates in exactly this order).
    pub dabs: Vec<Dab>,
    /// Union of dab pixel rects (pre-clip) — the contract `dirtyRect`
    /// basis ("D = union of dab bounds").
    pub bounds: IRect,
    /// The effective base diameter (`sizePx` override or preset diameter):
    /// the §15.3 `d_tip` used by the Δs clamp.
    pub base_diameter: f32,
}

/// Everything the brush engine can reject.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StrokeError {
    /// Input samples failed §15.1 validation.
    Samples(SampleBlobError),
    /// Preset failed §15.4/§15.7 validation.
    Preset(PresetError),
    /// `sizePx` outside 0.1..=10 000 (§10.3).
    SizePxOutOfRange(f32),
    /// `hardness` outside 0..=1 (§15.7).
    HardnessOutOfRange(f32),
    /// `opacity` outside 0..=1 (§10.3).
    OpacityOutOfRange(f32),
    /// `flow` outside 0..=1 (§10.3).
    FlowOutOfRange(f32),
    /// Paint color component not finite (§22.1 values are working-space
    /// linear; range policy is the storage boundary's).
    ColorNotFinite,
    /// Presentation asked for a non-`normal` §4.5 blend without a kernel
    /// table (the shared §4.5 math library arrives with O-RENDER; O-CORE
    /// passes it at wiring time).
    BlendKernelUnavailable(appname_core::BlendMode),
    /// The §11.1 TRANSPARENCY-lock constrain path (source-atop) is defined
    /// for `normal` in v1; other modes need the blend functions separated
    /// from their composites (kernel-table extension, lane log).
    TransparencyLockBlendUnsupported(appname_core::BlendMode),
}

impl core::fmt::Display for StrokeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StrokeError::Samples(e) => write!(f, "samples: {e}"),
            StrokeError::Preset(e) => write!(f, "preset: {e}"),
            StrokeError::SizePxOutOfRange(v) => write!(f, "sizePx {v} outside 0.1..=10000"),
            StrokeError::HardnessOutOfRange(v) => write!(f, "hardness {v} outside 0..=1"),
            StrokeError::OpacityOutOfRange(v) => write!(f, "opacity {v} outside 0..=1"),
            StrokeError::FlowOutOfRange(v) => write!(f, "flow {v} outside 0..=1"),
            StrokeError::ColorNotFinite => write!(f, "paint color not finite"),
            StrokeError::BlendKernelUnavailable(m) => {
                write!(
                    f,
                    "blend mode {m:?} needs a §4.5 kernel table (only normal is built in)"
                )
            }
            StrokeError::TransparencyLockBlendUnsupported(m) => {
                write!(
                    f,
                    "TRANSPARENCY-lock painting supports normal in v1, got {m:?}"
                )
            }
        }
    }
}

impl std::error::Error for StrokeError {}

impl From<SampleBlobError> for StrokeError {
    fn from(e: SampleBlobError) -> StrokeError {
        StrokeError::Samples(e)
    }
}

impl From<PresetError> for StrokeError {
    fn from(e: PresetError) -> StrokeError {
        StrokeError::Preset(e)
    }
}

// ---------------------------------------------------------------------
// Dynamics evaluation machinery
// ---------------------------------------------------------------------

/// Interpolated raw inputs at one march position (§15.3 dynamics lerp).
#[derive(Clone, Copy, Debug)]
struct RawInputs {
    pressure: f32,
    tilt_x: f32,
    tilt_y: f32,
    velocity: f32,
    rotation: f32,
    /// Stroke heading, radians (chord direction; +x when undefined).
    heading: [f64; 2],
    /// The per-dab `DynInput::Random` draw.
    random: f32,
}

const TAU: f32 = core::f32::consts::TAU;

impl RawInputs {
    /// §15.4 input normalization to `[0,1]`.
    fn normalized(&self, input: DynInput) -> f32 {
        match input {
            DynInput::Pressure => self.pressure.clamp(0.0, 1.0),
            DynInput::TiltX => (self.tilt_x + 1.0) * 0.5,
            DynInput::TiltY => (self.tilt_y + 1.0) * 0.5,
            DynInput::Velocity => (self.velocity / VELOCITY_NORM).min(1.0),
            DynInput::Rotation => self.rotation.rem_euclid(TAU) / TAU,
            DynInput::Random => self.random,
            DynInput::Direction => {
                let a = (self.heading[1].atan2(self.heading[0])) as f32;
                a.rem_euclid(TAU) / TAU
            }
        }
    }
}

/// One prebuilt curve: selector + interpolant + jitter.
struct BuiltCurve {
    input: DynInput,
    curve: MonotoneCurve,
    jitter: f32,
}

/// Prebuilt per-output curve lists + the frozen RNG bank.
struct DynEngine {
    outputs: [Vec<BuiltCurve>; 6],
    /// Streams 0–5 = `DynOutput` ordinal; index 6 = Random input stream.
    streams: [Pcg32; 7],
}

impl DynEngine {
    fn new(preset: &BrushPreset, seed: u64) -> DynEngine {
        let build = |out: DynOutput| -> Vec<BuiltCurve> {
            preset
                .dynamics
                .curves(out)
                .iter()
                .map(|c| BuiltCurve {
                    input: c.input,
                    curve: MonotoneCurve::build(&c.points).expect("preset validated"),
                    jitter: c.jitter,
                })
                .collect()
        };
        DynEngine {
            outputs: [
                build(DynOutput::Size),
                build(DynOutput::Opacity),
                build(DynOutput::Flow),
                build(DynOutput::Angle),
                build(DynOutput::Scatter),
                build(DynOutput::Roundness),
            ],
            streams: [
                Pcg32::new(seed, DynOutput::Size as u64),
                Pcg32::new(seed, DynOutput::Opacity as u64),
                Pcg32::new(seed, DynOutput::Flow as u64),
                Pcg32::new(seed, DynOutput::Angle as u64),
                Pcg32::new(seed, DynOutput::Scatter as u64),
                Pcg32::new(seed, DynOutput::Roundness as u64),
                Pcg32::new(seed, RANDOM_INPUT_STREAM),
            ],
        }
    }

    /// The unconditional per-march-position Random-input draw (module pin).
    fn draw_random_input(&mut self) -> f32 {
        self.streams[RANDOM_INPUT_STREAM as usize].u01()
    }

    /// §15.4 multiplier output: product of curve evaluations, then one
    /// jitter draw per curve with `jitter > 0` (curve order), clamped to
    /// `[0,1]`.
    fn eval_multiplier(&mut self, out: DynOutput, raw: &RawInputs) -> f32 {
        let k = out as usize;
        let mut v = 1.0f32;
        for c in &self.outputs[k] {
            v *= c.curve.eval(raw.normalized(c.input));
        }
        for j in 0..self.outputs[k].len() {
            let jit = self.outputs[k][j].jitter;
            if jit > 0.0 {
                v += self.streams[k].sym() * jit;
            }
        }
        v.clamp(0.0, 1.0)
    }

    /// §15.4 angle output: contributions **add** — `eval·2π` radians per
    /// curve, then jitter draws scaled by the angle range `2π` (module pin).
    fn eval_angle_offset(&mut self, raw: &RawInputs) -> f32 {
        let k = DynOutput::Angle as usize;
        let mut offset = 0.0f32;
        for c in &self.outputs[k] {
            offset += c.curve.eval(raw.normalized(c.input)) * TAU;
        }
        for j in 0..self.outputs[k].len() {
            let jit = self.outputs[k][j].jitter;
            if jit > 0.0 {
                offset += self.streams[k].sym() * jit * TAU;
            }
        }
        offset
    }

    /// §15.7 scatter draws (stream 4, after the scatter-output jitter):
    /// spawn count then per-spawn offsets. Returns per-spawn `[normal,
    /// tangent]` factors in `[−1,1]` (tangent 0 when `bothAxes` is off).
    fn draw_scatter(&mut self, preset: &BrushPreset, span: f32) -> Vec<[f32; 2]> {
        let k = DynOutput::Scatter as usize;
        let s = &preset.scatter;
        let mut n = s.count as u32;
        if s.count_jitter > 0.0 {
            let u = self.streams[k].u01();
            n = ((s.count as f32) * (1.0 - u * s.count_jitter)).round() as u32;
            n = n.max(1);
        }
        let mut spawns = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let mut factors = [0.0f32; 2];
            if span != 0.0 {
                factors[0] = self.streams[k].sym();
                if s.both_axes {
                    factors[1] = self.streams[k].sym();
                }
            }
            spawns.push(factors);
        }
        spawns
    }
}

// ---------------------------------------------------------------------
// The march
// ---------------------------------------------------------------------

/// Plan one stroke: validate everything, build the §15.3 path, march it,
/// and evaluate §15.4 dynamics per stamp position.
pub fn plan_stroke(
    samples: &[InputSample],
    preset: &BrushPreset,
    params: &StrokeParams,
) -> Result<StrokePlan, StrokeError> {
    validate_samples(samples)?;
    preset.validate()?;
    if let Some(s) = params.size_px {
        if !s.is_finite() || !(0.1..=10_000.0).contains(&s) {
            return Err(StrokeError::SizePxOutOfRange(s));
        }
    }
    if let Some(h) = params.hardness {
        if !h.is_finite() || !(0.0..=1.0).contains(&h) {
            return Err(StrokeError::HardnessOutOfRange(h));
        }
    }
    if !params.opacity.is_finite() || !(0.0..=1.0).contains(&params.opacity) {
        return Err(StrokeError::OpacityOutOfRange(params.opacity));
    }
    if !params.flow.is_finite() || !(0.0..=1.0).contains(&params.flow) {
        return Err(StrokeError::FlowOutOfRange(params.flow));
    }

    let base_diameter = params.size_px.unwrap_or(preset.diameter);
    // §15.7 tip hardness: command override or the preset's value, exactly as
    // `sizePx` falls back to the preset diameter above. Constant across the
    // stroke in v1 (not a §15.4 dynamics output).
    let hardness = params.hardness.unwrap_or(preset.hardness);
    let positions: Vec<[f64; 2]> = samples.iter().map(|s| [s.x, s.y]).collect();
    let kept = super::path::dedupe_coincident(&positions);
    let deduped: Vec<InputSample> = kept.iter().map(|&i| samples[i]).collect();
    let dedup_pos: Vec<[f64; 2]> = kept.iter().map(|&i| positions[i]).collect();
    let path = super::path::flatten(&dedup_pos);

    let mut engine = DynEngine::new(preset, params.seed);
    let mut dabs: Vec<Dab> = Vec::new();
    let mut bounds = IRect::EMPTY;

    // Dynamics lerp between the bracketing deduplicated raw samples at an
    // arc position (§15.3: positions from the spline, dynamics from this
    // lerp — never spline-interpolated).
    let lerp_dynamics = |arc: f64, heading: [f64; 2], random: f32| -> RawInputs {
        let sa = &path.sample_arc;
        // Find j with sa[j] <= arc <= sa[j+1] (sa is non-decreasing).
        let mut j = 0usize;
        while j + 2 < sa.len() && arc >= sa[j + 1] {
            j += 1;
        }
        let (a, b) = (&deduped[j], &deduped[(j + 1).min(deduped.len() - 1)]);
        let span = if j + 1 < sa.len() {
            sa[j + 1] - sa[j]
        } else {
            0.0
        };
        let u = if span > 0.0 {
            (((arc - sa[j]) / span).clamp(0.0, 1.0)) as f32
        } else {
            0.0
        };
        let lf = |x: f32, y: f32| x + (y - x) * u;
        RawInputs {
            pressure: lf(a.pressure, b.pressure),
            tilt_x: lf(a.tilt_x, b.tilt_x),
            tilt_y: lf(a.tilt_y, b.tilt_y),
            velocity: lf(a.velocity, b.velocity),
            rotation: lf(a.rotation, b.rotation),
            heading,
            random,
        }
    };

    // Emit the stamp(s) for one march position; returns Δs for the next
    // stamp (§15.3 clamp).
    let emit = |engine: &mut DynEngine,
                dabs: &mut Vec<Dab>,
                bounds: &mut IRect,
                arc: f64,
                pos: [f64; 2],
                heading: [f64; 2]|
     -> f64 {
        let random = engine.draw_random_input();
        let raw = lerp_dynamics(arc, heading, random);
        let size_mult = engine.eval_multiplier(DynOutput::Size, &raw);
        let opacity_mult = engine.eval_multiplier(DynOutput::Opacity, &raw);
        let flow_mult = engine.eval_multiplier(DynOutput::Flow, &raw);
        let angle_offset = engine.eval_angle_offset(&raw);
        let scatter_mult = engine.eval_multiplier(DynOutput::Scatter, &raw);
        let roundness_mult = engine.eval_multiplier(DynOutput::Roundness, &raw);

        // §15.4 output application (module pins for floor + ceiling).
        let floor = 1.0f32.min(base_diameter);
        let diameter = (base_diameter * size_mult).max(floor);
        let opacity = (params.opacity * opacity_mult).clamp(0.0, 1.0);
        let flow = (params.flow * flow_mult).clamp(0.0, 1.0);
        let angle_rad = preset.angle_rad + angle_offset;
        let roundness = (preset.roundness * roundness_mult).clamp(0.05, 1.0);
        let span = preset.scatter.amount * scatter_mult;

        let hlen = (heading[0] * heading[0] + heading[1] * heading[1]).sqrt();
        let unit = if hlen > 0.0 {
            [heading[0] / hlen, heading[1] / hlen]
        } else {
            [1.0, 0.0]
        };
        let normal = [-unit[1], unit[0]];

        for factors in engine.draw_scatter(preset, span) {
            let off_n = (factors[0] * span * diameter) as f64;
            let off_t = (factors[1] * span * diameter) as f64;
            let center = [
                pos[0] + normal[0] * off_n + unit[0] * off_t,
                pos[1] + normal[1] * off_n + unit[1] * off_t,
            ];
            let dab = Dab {
                center,
                diameter,
                angle_rad,
                roundness,
                hardness,
                flow,
                opacity,
            };
            *bounds = bounds.union(&dab.pixel_rect());
            dabs.push(dab);
        }

        // §15.3 [NORMATIVE] Δs clamp; d_prev = the just-emitted diameter.
        (preset.spacing as f64 * diameter as f64)
            .max(0.01 * base_diameter as f64)
            .max(0.25)
    };

    // First dab: immediately at the pen-down position with pen-down
    // dynamics (§15.3 — no dead zone; a single click paints one dab).
    let first_heading = if path.verts.len() >= 2 {
        let a = path.verts[0].pos;
        let b = path.verts[1].pos;
        [b[0] - a[0], b[1] - a[1]]
    } else {
        [1.0, 0.0] // v1 pin: +x when undefined
    };
    let mut delta_s = emit(
        &mut engine,
        &mut dabs,
        &mut bounds,
        0.0,
        path.verts[0].pos,
        first_heading,
    );
    let mut s_next = delta_s;
    let mut last_emit_arc = 0.0f64;

    // Arc-length walk of the flattened polyline (§15.3).
    for w in path.verts.windows(2) {
        let (va, vb) = (w[0], w[1]);
        let chord = [vb.pos[0] - va.pos[0], vb.pos[1] - va.pos[1]];
        let span = vb.arc - va.arc;
        if span <= 0.0 {
            continue;
        }
        while s_next <= vb.arc {
            let u = (s_next - va.arc) / span;
            let pos = [va.pos[0] + chord[0] * u, va.pos[1] + chord[1] * u];
            delta_s = emit(&mut engine, &mut dabs, &mut bounds, s_next, pos, chord);
            last_emit_arc = s_next;
            s_next += delta_s;
        }
    }

    // Final dab at the pen-up position iff the residual arc since the last
    // dab is ≥ 0.5·Δs (§15.3 — prevents the double-density end blob).
    if path.total > 0.0 {
        let residual = path.total - last_emit_arc;
        if residual >= 0.5 * delta_s {
            let end = path.verts[path.verts.len() - 1];
            let prev = path.verts[path.verts.len() - 2];
            let chord = [end.pos[0] - prev.pos[0], end.pos[1] - prev.pos[1]];
            emit(
                &mut engine,
                &mut dabs,
                &mut bounds,
                path.total,
                end.pos,
                chord,
            );
        }
    }

    Ok(StrokePlan {
        dabs,
        bounds,
        base_diameter,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::preset::{BrushDynamics, DynamicCurve, ScatterParams};
    use crate::brush::sample::InputSample;

    fn line_samples(n: usize, step: f64, pressure: impl Fn(usize) -> f32) -> Vec<InputSample> {
        (0..n)
            .map(|i| InputSample {
                x: i as f64 * step,
                y: 0.0,
                pressure: pressure(i),
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                velocity: 0.0,
                t: i as f64 * 0.008,
            })
            .collect()
    }

    fn constant_preset(diameter: f32, spacing: f32) -> BrushPreset {
        BrushPreset {
            diameter,
            spacing,
            hardness: 1.0,
            roundness: 1.0,
            angle_rad: 0.0,
            scatter: ScatterParams::default(),
            dynamics: BrushDynamics::default(),
        }
    }

    #[test]
    fn zero_length_stroke_is_exactly_one_dab() {
        // §15.3: single click = one dab at the pen-down position.
        let s = line_samples(1, 0.0, |_| 0.7);
        let plan = plan_stroke(&s, &constant_preset(20.0, 0.1), &StrokeParams::default()).unwrap();
        assert_eq!(plan.dabs.len(), 1);
        assert_eq!(plan.dabs[0].center, [0.0, 0.0]);
        // Coincident samples collapse to the same single dab.
        let s2 = vec![
            s[0],
            InputSample { t: 0.008, ..s[0] },
            InputSample { t: 0.016, ..s[0] },
        ];
        let plan2 =
            plan_stroke(&s2, &constant_preset(20.0, 0.1), &StrokeParams::default()).unwrap();
        assert_eq!(plan2.dabs.len(), 1);
    }

    #[test]
    fn straight_line_spacing_is_exact_and_speed_independent() {
        // 100 px line, diameter 20, spacing 0.5 → Δs = 10 px → dabs at
        // 0,10,…,100: 11 dabs (residual 0 < 0.5·Δs, no end dab).
        let p = constant_preset(20.0, 0.5);
        let s3 = line_samples(3, 50.0, |_| 1.0); // sparse events
        let s11 = line_samples(11, 10.0, |_| 1.0); // dense events
        let a = plan_stroke(&s3, &p, &StrokeParams::default()).unwrap();
        let b = plan_stroke(&s11, &p, &StrokeParams::default()).unwrap();
        assert_eq!(a.dabs.len(), 11, "exact spacing on the collinear path");
        assert_eq!(
            b.dabs.len(),
            11,
            "stamp positions are event-rate independent (§15.3 PERF)"
        );
        for (i, d) in a.dabs.iter().enumerate() {
            assert!(
                (d.center[0] - 10.0 * i as f64).abs() < 1e-6,
                "dab {i} at {}",
                d.center[0]
            );
            assert!(d.center[1].abs() < 1e-9);
        }
        for (x, y) in a.dabs.iter().zip(b.dabs.iter()) {
            assert!((x.center[0] - y.center[0]).abs() < 1e-6);
        }
    }

    #[test]
    fn end_dab_rule_half_delta_s() {
        // 104 px line, Δs = 10: last march dab at 100, residual 4 < 5 → no
        // end dab. 107 px: residual 7 ≥ 5 → end dab at 107.
        let p = constant_preset(20.0, 0.5);
        let mk = |len: f64| {
            vec![
                InputSample {
                    x: 0.0,
                    y: 0.0,
                    pressure: 1.0,
                    tilt_x: 0.0,
                    tilt_y: 0.0,
                    rotation: 0.0,
                    velocity: 0.0,
                    t: 0.0,
                },
                InputSample {
                    x: len,
                    y: 0.0,
                    pressure: 1.0,
                    tilt_x: 0.0,
                    tilt_y: 0.0,
                    rotation: 0.0,
                    velocity: 0.0,
                    t: 0.1,
                },
            ]
        };
        let a = plan_stroke(&mk(104.0), &p, &StrokeParams::default()).unwrap();
        assert_eq!(a.dabs.len(), 11, "residual below half-Δs is skipped");
        assert!((a.dabs.last().unwrap().center[0] - 100.0).abs() < 1e-6);
        let b = plan_stroke(&mk(107.0), &p, &StrokeParams::default()).unwrap();
        assert_eq!(
            b.dabs.len(),
            12,
            "residual at/over half-Δs emits the pen-up dab"
        );
        assert!((b.dabs.last().unwrap().center[0] - 107.0).abs() < 1e-6);
    }

    #[test]
    fn min_spacing_clamp_holds() {
        // Tiny spacing: Δs = max(spacing·d, 0.01·d_tip, 0.25). d = 10 →
        // 0.01·d_tip = 0.1 → floor is 0.25 px.
        let p = constant_preset(10.0, 0.001);
        let s = line_samples(2, 10.0, |_| 1.0);
        let plan = plan_stroke(&s, &p, &StrokeParams::default()).unwrap();
        // 10 px / 0.25 = 40 intervals + first dab = 41 (+ possible end dab).
        assert!(
            plan.dabs.len() >= 41 && plan.dabs.len() <= 42,
            "got {}",
            plan.dabs.len()
        );
        let d01 = plan.dabs[1].center[0] - plan.dabs[0].center[0];
        assert!((d01 - 0.25).abs() < 1e-6, "clamped step, got {d01}");
        // Large tip: 0.01·d_tip dominates. d_tip = 100 → Δs = 1.
        let p = constant_preset(100.0, 0.001);
        let plan = plan_stroke(
            &line_samples(2, 10.0, |_| 1.0),
            &p,
            &StrokeParams::default(),
        )
        .unwrap();
        let d01 = plan.dabs[1].center[0] - plan.dabs[0].center[0];
        assert!((d01 - 1.0).abs() < 1e-6, "1%-of-tip clamp, got {d01}");
    }

    #[test]
    fn pressure_size_dynamics_follow_the_ramp_and_spacing_follows_size() {
        let mut p = constant_preset(20.0, 0.5);
        p.dynamics = BrushDynamics {
            size: vec![DynamicCurve::pressure_linear()],
            ..BrushDynamics::default()
        };
        let s = line_samples(2, 100.0, |i| if i == 0 { 0.0 } else { 1.0 });
        let plan = plan_stroke(&s, &p, &StrokeParams::default()).unwrap();
        // Pressure ramps 0→1 along the arc; diameter must be non-decreasing
        // and spacing (gap to next dab) must track the emitted diameter
        // (§15.3: Δs from d_prev, one stamp behind).
        for w in plan.dabs.windows(2) {
            assert!(w[1].diameter >= w[0].diameter - 1e-6);
            let gap = w[1].center[0] - w[0].center[0];
            let want = (0.5 * w[0].diameter as f64).max(0.2).max(0.25);
            assert!((gap - want).abs() < 1e-5, "gap {gap} vs Δs(d_prev) {want}");
        }
        // Zero pressure with a linear size curve floors at min(1px, base).
        assert!((plan.dabs[0].diameter - 1.0).abs() < 1e-6);
        // The last dab sits within one Δs of the stroke end, so pressure is
        // near (not exactly) 1 there.
        let last = plan.dabs.last().unwrap().diameter;
        assert!(
            (19.0..=20.0).contains(&last),
            "near-full pressure at stroke end, got {last}"
        );
    }

    #[test]
    fn sub_pixel_base_keeps_its_size_under_the_floor_pin() {
        let p = constant_preset(0.5, 0.5);
        let s = line_samples(2, 5.0, |_| 1.0);
        let plan = plan_stroke(&s, &p, &StrokeParams::default()).unwrap();
        assert!(
            (plan.dabs[0].diameter - 0.5).abs() < 1e-6,
            "floor is min(1px, base)"
        );
    }

    #[test]
    fn size_px_override_replaces_preset_diameter() {
        let p = constant_preset(64.0, 0.5);
        let s = line_samples(2, 40.0, |_| 1.0);
        let params = StrokeParams {
            size_px: Some(10.0),
            ..StrokeParams::default()
        };
        let plan = plan_stroke(&s, &p, &params).unwrap();
        assert_eq!(plan.base_diameter, 10.0);
        assert!((plan.dabs[0].diameter - 10.0).abs() < 1e-6);
        // Δs = 0.5·10 = 5 → 9 dabs over 40 px.
        assert_eq!(plan.dabs.len(), 9);
    }

    #[test]
    fn hardness_override_replaces_preset_hardness() {
        // Symmetric with `size_px_override_replaces_preset_diameter`: the
        // command `hardness` reaches `Dab.hardness` (§15.5 tip_coverage),
        // and its absence falls back to the preset's value byte-for-byte.
        let mut p = constant_preset(20.0, 0.5);
        p.hardness = 0.2; // distinct from the override so the swap is visible
        let s = line_samples(2, 10.0, |_| 1.0);

        let overridden = plan_stroke(
            &s,
            &p,
            &StrokeParams {
                hardness: Some(0.8),
                ..StrokeParams::default()
            },
        )
        .unwrap();
        assert!(overridden
            .dabs
            .iter()
            .all(|d| (d.hardness - 0.8).abs() < 1e-6));

        // Absent override → every dab carries the preset hardness, identical
        // to the default-params plan (determinism: no override, no change).
        let fallback = plan_stroke(&s, &p, &StrokeParams::default()).unwrap();
        assert!(fallback
            .dabs
            .iter()
            .all(|d| (d.hardness - 0.2).abs() < 1e-6));
        assert_eq!(
            fallback.dabs,
            plan_stroke(&s, &p, &StrokeParams::default()).unwrap().dabs,
            "absent hardness ⇒ preset default, byte-identical replays"
        );
        // The override genuinely changes the stamp sequence.
        assert_ne!(overridden.dabs, fallback.dabs);
    }

    #[test]
    fn param_validation_rejects() {
        let p = constant_preset(20.0, 0.5);
        let s = line_samples(2, 10.0, |_| 1.0);
        assert_eq!(
            plan_stroke(
                &s,
                &p,
                &StrokeParams {
                    size_px: Some(0.05),
                    ..Default::default()
                }
            )
            .unwrap_err(),
            StrokeError::SizePxOutOfRange(0.05)
        );
        // Hardness override out of the §15.7 [0,1] domain → typed refusal
        // (mirrors the sizePx rejection above).
        assert_eq!(
            plan_stroke(
                &s,
                &p,
                &StrokeParams {
                    hardness: Some(1.5),
                    ..Default::default()
                }
            )
            .unwrap_err(),
            StrokeError::HardnessOutOfRange(1.5)
        );
        assert!(matches!(
            plan_stroke(
                &s,
                &p,
                &StrokeParams {
                    hardness: Some(f32::NAN),
                    ..Default::default()
                }
            )
            .unwrap_err(),
            StrokeError::HardnessOutOfRange(_)
        ));
        assert_eq!(
            plan_stroke(
                &s,
                &p,
                &StrokeParams {
                    opacity: 1.5,
                    ..Default::default()
                }
            )
            .unwrap_err(),
            StrokeError::OpacityOutOfRange(1.5)
        );
        assert_eq!(
            plan_stroke(
                &s,
                &p,
                &StrokeParams {
                    flow: -0.1,
                    ..Default::default()
                }
            )
            .unwrap_err(),
            StrokeError::FlowOutOfRange(-0.1)
        );
        assert!(matches!(
            plan_stroke(&[], &p, &StrokeParams::default()),
            Err(StrokeError::Samples(_))
        ));
        let mut bad = p.clone();
        bad.hardness = 2.0;
        assert!(matches!(
            plan_stroke(&s, &bad, &StrokeParams::default()),
            Err(StrokeError::Preset(_))
        ));
    }

    #[test]
    fn replay_is_bit_identical_and_seed_sensitive() {
        let mut p = constant_preset(24.0, 0.3);
        p.dynamics.size = vec![DynamicCurve {
            input: DynInput::Random,
            points: vec![[0.0, 0.2], [1.0, 1.0]],
            jitter: 0.25,
        }];
        p.scatter = ScatterParams {
            amount: 0.5,
            both_axes: true,
            count: 3,
            count_jitter: 0.5,
        };
        let s = line_samples(6, 15.0, |i| (i as f32 / 5.0).min(1.0));
        let params = StrokeParams {
            seed: 0xDEADBEEF,
            ..StrokeParams::default()
        };
        let a = plan_stroke(&s, &p, &params).unwrap();
        let b = plan_stroke(&s, &p, &params).unwrap();
        assert_eq!(
            a.dabs, b.dabs,
            "same seed ⇒ bit-identical dab sequence (§34.4)"
        );
        let c = plan_stroke(
            &s,
            &p,
            &StrokeParams {
                seed: 0xDEADBEF0,
                ..params
            },
        )
        .unwrap();
        assert_ne!(a.dabs, c.dabs, "different seed ⇒ different dynamics draws");
    }

    #[test]
    fn scatter_spawns_between_one_and_count_dabs_per_position() {
        let mut p = constant_preset(20.0, 1.0);
        p.scatter = ScatterParams {
            amount: 1.0,
            both_axes: false,
            count: 4,
            count_jitter: 1.0,
        };
        let s = line_samples(2, 100.0, |_| 1.0);
        let plan = plan_stroke(
            &s,
            &p,
            &StrokeParams {
                seed: 7,
                ..Default::default()
            },
        )
        .unwrap();
        // 6 march positions (0,20,…,100); spawns per position ∈ 1..=4.
        assert!(
            plan.dabs.len() >= 6 && plan.dabs.len() <= 24,
            "got {}",
            plan.dabs.len()
        );
        // Scatter with amount 1 must move dabs off the y=0 axis (normal is ±y).
        assert!(plan.dabs.iter().any(|d| d.center[1].abs() > 0.01));
    }

    #[test]
    fn no_scatter_and_no_jitter_consume_no_optional_draws() {
        // A preset with empty dynamics must produce identical dabs whatever
        // the seed (only the unconditional Random-input draw advances, and
        // nothing consumes its value).
        let p = constant_preset(16.0, 0.5);
        let s = line_samples(3, 20.0, |_| 0.8);
        let a = plan_stroke(
            &s,
            &p,
            &StrokeParams {
                seed: 1,
                ..Default::default()
            },
        )
        .unwrap();
        let b = plan_stroke(
            &s,
            &p,
            &StrokeParams {
                seed: 2,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(a.dabs, b.dabs, "constant preset is seed-independent");
    }

    #[test]
    fn bounds_cover_every_dab_rect() {
        let p = constant_preset(30.0, 0.5);
        let s = line_samples(4, 25.0, |_| 1.0);
        let plan = plan_stroke(&s, &p, &StrokeParams::default()).unwrap();
        for d in &plan.dabs {
            assert!(plan.bounds.contains_rect(&d.pixel_rect()));
        }
        // Sanity: a 30 px dab at (0,0) reaches ≈ ±16 px.
        assert!(plan.bounds.min.x <= -15 && plan.bounds.min.x >= -18);
    }
}
