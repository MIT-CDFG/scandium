// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §16.3 "Liquify — `filter.liquify`" + §45.4 "Liquify workspace
//! contract" [NORMATIVE]; DD ⑯ (the commit is a §16.1 registry filter —
//! `appname-filters/src/corpus/liquify.rs` consumes this module's pure
//! engines); plan §2.5 warps row, "liquify workspace (forward-warp grid,
//! era/expired basis per register)".
//!
//! Rights basis (V39 select-and-screen discipline, no release claim):
//! contract `contracts/algorithms/algorithms.yaml` area `warps_upright`
//! ("Liquify: forward-warp displacement brushes … on the displacement-grid
//! contract SPEC §45.4 already binds — generic mesh warping (Wolberg-era,
//! L0)"); register key P-7312805 (US7312805B1, "User defined warping
//! tool", Adobe) — quoted status "Expired - Lifetime", expiry 2024-04-06,
//! verified 2026-07-24, `clears: [liquify.forward_warp_workspace]`
//! (machine field name quoted from the register). Every formula below is
//! the §16.3/§45.4 normative text transcribed.
//!
//! WHAT THIS MODULE IS: the tool-side liquify workspace machinery —
//! * the persistent displacement field `D` (§45.4 field representation:
//!   f32 (dx, dy) on a regular grid of pitch ∈ {4, 8, 16, 32} covering
//!   the LAYER bounds, bilinear between nodes, holding the BACKWARD map
//!   `out(p) = resample_bicubic(src, p + D(p))`);
//! * the six §45.4 v1 brushes with the Alt inverter mapping and the
//!   density-shaped falloff `f' = f^(2·(1−density))`,
//!   `f = (1 − (d/R)²)²`;
//! * the §45.4 canvas-edge rule (boundary nodes pinned to `D = 0`);
//! * the `.bmesh` (`appname.mesh.v1`) strict codec (§45.4 load
//!   validation: schema id, pitch set, finite floats, exact disp length —
//!   violations reject and leave the current field untouched);
//! * the commit resample engine (one full-res §18.2 bicubic (Catmull-Rom)
//!   pass through `D`, premultiplied, outside-canvas ⇒ transparent).
//!
//! The workspace SHELL (modal dialog, 60 Hz tick loop, backdrop
//! rendering, mesh overlay, Restore All button) is O-UI's §32.5 surface;
//! preview GPU residency (`rg32float` texture) is O-RENDER's. This module
//! is the deterministic CPU truth both consume.
//!
//! v1 pins (recorded, not invented — lane log this date):
//! * Field evaluation point: `D` is evaluated at the OUTPUT PIXEL CENTER
//!   `p = (x + 0.5, y + 0.5)`; the source sample position `p + D(p)` maps
//!   back to sample space with the same −0.5 center shift the §18.4 grid
//!   warp uses (`warp.rs` precedent). An all-zero field therefore lands
//!   every tap on an exact integer offset where the Catmull-Rom weights
//!   are exactly {0, 0, 1, 0, 0} — the §45.8 `prop_liquify_identity`
//!   bit-identity holds by construction (tested, not assumed).
//! * Sampling OUTSIDE the node extent: `D = 0` (the field covers the
//!   layer bounds per §45.4; §16.3's remap outside the grid is the
//!   un-displaced read). Node values are stored UNCLAMPED f32 (§45.4
//!   "a node may point anywhere"); only sampling applies the
//!   outside-canvas ⇒ transparent policy.
//! * Canvas-edge pinning: a node whose DOC position lies on or beyond
//!   any canvas boundary LINE (`pos.x ≤ canvas.min.x`,
//!   `pos.x ≥ canvas.max.x`, same for y) is skipped by every brush
//!   update — §45.4 "nodes lying on the canvas boundary are pinned to
//!   D = 0 — brush updates skip them".
//! * Rate constants: ω = 0.05 rad/tick (twirl), κ = 0.05/tick
//!   (pucker/bloat), reconstruct rate 0.1·s·f′/tick — §16.3 verbatim;
//!   the 60 Hz tick SCHEDULING is workspace-shell state, this module
//!   exposes the per-event/per-tick update only.
//! * `.bmesh` strictness: unknown top-level keys REJECT (the §45.4 v2
//!   `freeze` extension arrives under a NEW schema id `appname.mesh.v2`
//!   that v1 readers reject by id — accepting stray keys in v1 would
//!   make that gate leaky). Recorded as a pin; `bmesh_parse` is the
//!   §34.2 fuzz-target shape (§45.8 `fuzz_bmesh_parse` — hostile inputs
//!   answer typed errors, never panic/OOM).
//! * `fit_to` (load onto a different-sized target) scales node POSITIONS
//!   proportionally, never displacements (§45.4/§16.3) — realized as
//!   per-axis node SPACING factors on the in-memory field; serialization
//!   always writes the canonical integer pitch (a fitted field is
//!   workspace state, not a wire form).

use appname_core::IRect;
use serde_json::{json, Value};

use crate::resample::{EdgePolicy, Kernel, Plane};

/// §45.4: the legal mesh pitches.
pub const LIQUIFY_PITCHES: [i32; 4] = [4, 8, 16, 32];

/// §16.3 brush radius range (px).
pub const LIQUIFY_RADIUS_RANGE: core::ops::RangeInclusive<f64> = 1.0..=2000.0;

/// §16.3 rate constants (per 60 Hz tick).
pub const TWIRL_OMEGA: f64 = 0.05;
pub const PINCH_KAPPA: f64 = 0.05;
pub const RECONSTRUCT_RATE: f64 = 0.1;

/// The §45.4 persistent displacement field `D`.
#[derive(Clone, Debug, PartialEq)]
pub struct LiquifyField {
    /// §45.4 mesh pitch, px (∈ {4, 8, 16, 32}).
    pub pitch: i32,
    /// Doc-space position of node (0, 0) — the layer bounds min at
    /// workspace entry.
    pub origin: [i32; 2],
    /// Nodes per axis (`⌈extent/pitch⌉ + 1` over the layer bounds).
    pub nodes_x: usize,
    /// Nodes per axis.
    pub nodes_y: usize,
    /// Row-major (dx, dy) px per node, unclamped f32 (§45.4).
    pub disp: Vec<[f32; 2]>,
    /// Per-axis node spacing in doc px. `[pitch, pitch]` for a native
    /// field; `fit_to` rescales POSITIONS by adjusting this (module pin).
    pub spacing: [f64; 2],
}

/// Field construction / codec failures — typed, never a panic (§45.4
/// load validation; §34.2 fuzz-target discipline).
#[derive(Clone, Debug, PartialEq)]
pub enum LiquifyError {
    /// pitch ∉ {4, 8, 16, 32}.
    PitchInvalid(i64),
    /// The covered bounds are empty.
    EmptyBounds,
    /// `.bmesh` schema id is not `appname.mesh.v1` (v2+ rejects here by
    /// design, §45.4).
    SchemaId(String),
    /// `len(disp) != 2·size.x·size.y` (§45.4).
    DispLength { expected: usize, actual: usize },
    /// A float is NaN/±Inf (§0.3 `non_finite`).
    NonFinite(&'static str),
    /// Structural JSON violation (wrong type, missing key, unknown key,
    /// out-of-range integer).
    Malformed(&'static str),
}

impl core::fmt::Display for LiquifyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LiquifyError::PitchInvalid(p) => {
                write!(f, "meshPitch {p} outside {{4, 8, 16, 32}} (§45.4)")
            }
            LiquifyError::EmptyBounds => write!(f, "field bounds are empty (§45.4)"),
            LiquifyError::SchemaId(s) => {
                write!(
                    f,
                    "schema {s:?} is not appname.mesh.v1 (§45.4 reject-by-id)"
                )
            }
            LiquifyError::DispLength { expected, actual } => write!(
                f,
                "disp length {actual} != 2·size.x·size.y = {expected} (§45.4)"
            ),
            LiquifyError::NonFinite(k) => write!(f, "{k}: non-finite float (§0.3)"),
            LiquifyError::Malformed(m) => write!(f, "malformed .bmesh: {m} (§45.4)"),
        }
    }
}

impl std::error::Error for LiquifyError {}

impl LiquifyField {
    /// A zero field over `layer_bounds` at `pitch` (§45.4 node counts:
    /// `⌈extent/pitch⌉ + 1` per axis).
    pub fn new(layer_bounds: IRect, pitch: i32) -> Result<LiquifyField, LiquifyError> {
        if !LIQUIFY_PITCHES.contains(&pitch) {
            return Err(LiquifyError::PitchInvalid(i64::from(pitch)));
        }
        if layer_bounds.is_empty() {
            return Err(LiquifyError::EmptyBounds);
        }
        let nx = (layer_bounds.width() as usize).div_ceil(pitch as usize) + 1;
        let ny = (layer_bounds.height() as usize).div_ceil(pitch as usize) + 1;
        Ok(LiquifyField {
            pitch,
            origin: [layer_bounds.min.x, layer_bounds.min.y],
            nodes_x: nx,
            nodes_y: ny,
            disp: vec![[0.0; 2]; nx * ny],
            spacing: [f64::from(pitch), f64::from(pitch)],
        })
    }

    /// Doc-space position of node (i, j).
    pub fn node_pos(&self, i: usize, j: usize) -> [f64; 2] {
        [
            f64::from(self.origin[0]) + i as f64 * self.spacing[0],
            f64::from(self.origin[1]) + j as f64 * self.spacing[1],
        ]
    }

    /// Bilinear field sample at doc coords; outside the node extent the
    /// displacement is zero (module pin).
    pub fn sample(&self, px: f64, py: f64) -> [f64; 2] {
        let gx = (px - f64::from(self.origin[0])) / self.spacing[0];
        let gy = (py - f64::from(self.origin[1])) / self.spacing[1];
        if gx < 0.0 || gy < 0.0 || gx > (self.nodes_x - 1) as f64 || gy > (self.nodes_y - 1) as f64
        {
            return [0.0, 0.0];
        }
        let x0 = gx.floor();
        let y0 = gy.floor();
        let fx = gx - x0;
        let fy = gy - y0;
        let i0 = x0 as usize;
        let j0 = y0 as usize;
        let i1 = (i0 + 1).min(self.nodes_x - 1);
        let j1 = (j0 + 1).min(self.nodes_y - 1);
        let at = |i: usize, j: usize| self.disp[j * self.nodes_x + i];
        let mut out = [0.0f64; 2];
        for (c, o) in out.iter_mut().enumerate() {
            let v00 = f64::from(at(i0, j0)[c]);
            let v10 = f64::from(at(i1, j0)[c]);
            let v01 = f64::from(at(i0, j1)[c]);
            let v11 = f64::from(at(i1, j1)[c]);
            *o = v00 * (1.0 - fx) * (1.0 - fy)
                + v10 * fx * (1.0 - fy)
                + v01 * (1.0 - fx) * fy
                + v11 * fx * fy;
        }
        out
    }

    /// Max per-axis |displacement| over the nodes (the commit filter's
    /// ROI basis).
    pub fn max_disp(&self) -> f64 {
        self.disp
            .iter()
            .map(|d| f64::from(d[0].abs()).max(f64::from(d[1].abs())))
            .fold(0.0, f64::max)
    }

    /// §45.4 "Restore All": `D ← 0` everywhere (workspace state, not a
    /// document command — the caller is the workspace shell).
    pub fn restore_all(&mut self) {
        for d in &mut self.disp {
            *d = [0.0; 2];
        }
    }

    /// §45.4 mesh load onto a different-sized target: node POSITIONS
    /// scale proportionally onto `target_bounds`, displacements are
    /// untouched (module pin: realized as per-axis spacing factors).
    pub fn fit_to(&self, target_bounds: IRect) -> Result<LiquifyField, LiquifyError> {
        if target_bounds.is_empty() {
            return Err(LiquifyError::EmptyBounds);
        }
        let mut out = self.clone();
        out.origin = [target_bounds.min.x, target_bounds.min.y];
        let src_w = (self.nodes_x - 1) as f64 * self.spacing[0];
        let src_h = (self.nodes_y - 1) as f64 * self.spacing[1];
        if src_w > 0.0 {
            out.spacing[0] = self.spacing[0] * f64::from(target_bounds.width()) / src_w;
        }
        if src_h > 0.0 {
            out.spacing[1] = self.spacing[1] * f64::from(target_bounds.height()) / src_h;
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Brushes (§45.4 "Brush set — decided").
// ---------------------------------------------------------------------------

/// The §45.4 v1 brush set (exactly §16.3's six). The Alt inverter acts
/// where a mirror-twin exists (§45.4 table): Twirl flips sign, Pucker
/// acts as Bloat and vice versa, Push Left pushes right; Forward Warp
/// and Reconstruct have no twin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiquifyBrush {
    /// `D(x) ← D(x) − s·f·Δc`.
    ForwardWarp,
    /// `D(x) ← D(x)·(1 − 0.1·s·f)` per tick (relax toward identity).
    Reconstruct,
    /// `D(x) ← D(x) + s·f·ω·perp(x − c)` — clockwise (y-down);
    /// Alt ⇒ counterclockwise.
    Twirl,
    /// `D(x) ← D(x) + s·f·κ·(x − c)` (samples outward ⇒ content
    /// contracts); Alt ⇒ acts as Bloat.
    Pucker,
    /// `D(x) ← D(x) − s·f·κ·(x − c)` (samples inward ⇒ content
    /// expands); Alt ⇒ acts as Pucker.
    Bloat,
    /// `D(x) ← D(x) + s·f·|Δc|·perp(Δĉ)`, `perp(v) = (−v.y, v.x)`;
    /// Alt ⇒ push right (sign flip).
    PushLeft,
}

/// §45.4 per-brush options (§32.5 size/pressure/density row).
#[derive(Clone, Copy, Debug)]
pub struct BrushOptions {
    /// Size `R`, 1–2000 px.
    pub radius: f64,
    /// Strength `s` ∈ [0, 1] (the 100 %-pressure slider value; stylus
    /// pressure pre-scales it through the §15.4 curve on the tool side).
    pub strength: f64,
    /// Density ∈ [0, 1], default 0.5: falloff exponent shaping
    /// `f' = f^(2·(1−density))` — 0.5 is §16.3's exact base `f`.
    pub density: f64,
    /// Alt held (§45.4 in-place inverter).
    pub alt: bool,
}

impl Default for BrushOptions {
    fn default() -> BrushOptions {
        BrushOptions {
            radius: 100.0,
            strength: 0.5,
            density: 0.5,
            alt: false,
        }
    }
}

/// One pointer event / rate tick against the field.
#[derive(Clone, Copy, Debug)]
pub struct BrushEvent {
    /// Cursor `c` (doc px).
    pub cursor: [f64; 2],
    /// Cursor delta `Δc` since the previous event (zero for the held
    /// rate ticks of brushes 2–5).
    pub delta: [f64; 2],
}

/// §45.4 base falloff `f = (1 − (d/R)²)²` shaped by density
/// `f' = f^(2·(1−density))`; zero at `d ≥ R`.
pub fn falloff(d: f64, radius: f64, density: f64) -> f64 {
    if d >= radius {
        return 0.0;
    }
    let t = d / radius;
    let f = (1.0 - t * t).powi(2);
    let expo = 2.0 * (1.0 - density.clamp(0.0, 1.0));
    if expo == 0.0 {
        // density = 1: flat-hard inside the brush (§45.4).
        1.0
    } else {
        f.powf(expo)
    }
}

fn perp(v: [f64; 2]) -> [f64; 2] {
    [-v[1], v[0]]
}

/// Apply one brush event to the field (§16.3 update rules verbatim, Alt
/// mapping per §45.4). Canvas-boundary nodes are pinned (skipped). The
/// options are assumed pre-validated by the tool shell; radius/strength
/// are clamped defensively to their §16.3 ranges.
pub fn apply_brush_event(
    field: &mut LiquifyField,
    brush: LiquifyBrush,
    opts: &BrushOptions,
    event: BrushEvent,
    canvas: IRect,
) {
    let radius = opts.radius.clamp(1.0, 2000.0);
    let s = opts.strength.clamp(0.0, 1.0);
    let c = event.cursor;
    let dc = event.delta;
    // Push-direction unit vector (guarded: a zero-length drag pushes
    // nothing).
    let dc_len = (dc[0] * dc[0] + dc[1] * dc[1]).sqrt();
    for j in 0..field.nodes_y {
        for i in 0..field.nodes_x {
            let pos = field.node_pos(i, j);
            // §45.4 edge behavior: canvas-boundary nodes pinned to D = 0.
            if pos[0] <= f64::from(canvas.min.x)
                || pos[0] >= f64::from(canvas.max.x)
                || pos[1] <= f64::from(canvas.min.y)
                || pos[1] >= f64::from(canvas.max.y)
            {
                continue;
            }
            let dx = pos[0] - c[0];
            let dy = pos[1] - c[1];
            let d = (dx * dx + dy * dy).sqrt();
            let f = falloff(d, radius, opts.density);
            if f == 0.0 {
                continue;
            }
            let sf = s * f;
            let node = &mut field.disp[j * field.nodes_x + i];
            let cur = [f64::from(node[0]), f64::from(node[1])];
            let next: [f64; 2] = match brush {
                LiquifyBrush::ForwardWarp => [cur[0] - sf * dc[0], cur[1] - sf * dc[1]],
                LiquifyBrush::Reconstruct => {
                    let k = 1.0 - RECONSTRUCT_RATE * sf;
                    [cur[0] * k, cur[1] * k]
                }
                LiquifyBrush::Twirl => {
                    let p = perp([dx, dy]);
                    let sign = if opts.alt { -1.0 } else { 1.0 };
                    [
                        cur[0] + sign * sf * TWIRL_OMEGA * p[0],
                        cur[1] + sign * sf * TWIRL_OMEGA * p[1],
                    ]
                }
                LiquifyBrush::Pucker | LiquifyBrush::Bloat => {
                    // Pucker: +κ·(x − c); Bloat: −κ·(x − c); Alt swaps
                    // the two (§45.4).
                    let base_sign = if brush == LiquifyBrush::Pucker {
                        1.0
                    } else {
                        -1.0
                    };
                    let sign = if opts.alt { -base_sign } else { base_sign };
                    [
                        cur[0] + sign * sf * PINCH_KAPPA * dx,
                        cur[1] + sign * sf * PINCH_KAPPA * dy,
                    ]
                }
                LiquifyBrush::PushLeft => {
                    if dc_len == 0.0 {
                        cur
                    } else {
                        let hat = [dc[0] / dc_len, dc[1] / dc_len];
                        let p = perp(hat);
                        let sign = if opts.alt { -1.0 } else { 1.0 };
                        [
                            cur[0] + sign * sf * dc_len * p[0],
                            cur[1] + sign * sf * dc_len * p[1],
                        ]
                    }
                }
            };
            *node = [next[0] as f32, next[1] as f32];
        }
    }
}

// ---------------------------------------------------------------------------
// `.bmesh` codec (§16.3 / §45.4 `appname.mesh.v1`).
// ---------------------------------------------------------------------------

/// The `appname.mesh.v1` schema id.
pub const BMESH_SCHEMA: &str = "appname.mesh.v1";

/// Serialize the field as the §16.3 `.bmesh` JSON document. Always
/// writes the canonical integer pitch; a `fit_to`-rescaled field
/// serializes its ORIGINAL pitch (module pin — the fitted spacing is
/// workspace state).
pub fn to_bmesh(field: &LiquifyField) -> String {
    let mut disp = Vec::with_capacity(field.disp.len() * 2);
    for d in &field.disp {
        disp.push(json!(d[0]));
        disp.push(json!(d[1]));
    }
    json!({
        "schema": BMESH_SCHEMA,
        "pitch": field.pitch,
        "origin": [field.origin[0], field.origin[1]],
        "size": [field.nodes_x, field.nodes_y],
        "disp": disp,
    })
    .to_string()
}

fn bmesh_i64(v: &Value, what: &'static str) -> Result<i64, LiquifyError> {
    v.as_i64().ok_or(LiquifyError::Malformed(what))
}

/// Parse a `.bmesh` document under the §45.4 [NORMATIVE] validation
/// rules: schema id, `pitch ∈ {4, 8, 16, 32}`, finite floats,
/// `len(disp) == 2·size.x·size.y`. Violations answer a typed error and
/// the caller's current field stays untouched (this function never
/// mutates anything). Unknown top-level keys reject (module pin). This
/// is the §45.8 `fuzz_bmesh_parse` target shape: hostile inputs must
/// never panic or over-allocate (`disp` is bounded by the parsed input
/// itself; `size` is cross-checked against it before any allocation).
pub fn parse_bmesh(text: &str) -> Result<LiquifyField, LiquifyError> {
    let v: Value =
        serde_json::from_str(text).map_err(|_| LiquifyError::Malformed("not valid JSON"))?;
    let obj = v
        .as_object()
        .ok_or(LiquifyError::Malformed("top level is not an object"))?;
    for k in obj.keys() {
        if !matches!(k.as_str(), "schema" | "pitch" | "origin" | "size" | "disp") {
            return Err(LiquifyError::Malformed("unknown top-level key"));
        }
    }
    let schema = obj
        .get("schema")
        .and_then(Value::as_str)
        .ok_or(LiquifyError::Malformed("schema id missing"))?;
    if schema != BMESH_SCHEMA {
        return Err(LiquifyError::SchemaId(schema.to_string()));
    }
    let pitch = bmesh_i64(
        obj.get("pitch").ok_or(LiquifyError::Malformed("pitch"))?,
        "pitch is not an integer",
    )?;
    if !LIQUIFY_PITCHES.contains(&(pitch as i32)) || i32::try_from(pitch).is_err() {
        return Err(LiquifyError::PitchInvalid(pitch));
    }
    let origin = obj
        .get("origin")
        .and_then(Value::as_array)
        .ok_or(LiquifyError::Malformed("origin"))?;
    if origin.len() != 2 {
        return Err(LiquifyError::Malformed("origin is not [x, y]"));
    }
    let ox = bmesh_i64(&origin[0], "origin.x is not an integer")?;
    let oy = bmesh_i64(&origin[1], "origin.y is not an integer")?;
    let (ox, oy) = (
        i32::try_from(ox).map_err(|_| LiquifyError::Malformed("origin.x out of i32 range"))?,
        i32::try_from(oy).map_err(|_| LiquifyError::Malformed("origin.y out of i32 range"))?,
    );
    let size = obj
        .get("size")
        .and_then(Value::as_array)
        .ok_or(LiquifyError::Malformed("size"))?;
    if size.len() != 2 {
        return Err(LiquifyError::Malformed("size is not [nx, ny]"));
    }
    let sx = bmesh_i64(&size[0], "size.x is not an integer")?;
    let sy = bmesh_i64(&size[1], "size.y is not an integer")?;
    if sx < 1 || sy < 1 {
        return Err(LiquifyError::Malformed("size axes must be >= 1"));
    }
    let (sx, sy) = (sx as usize, sy as usize);
    let disp = obj
        .get("disp")
        .and_then(Value::as_array)
        .ok_or(LiquifyError::Malformed("disp"))?;
    let expected = sx
        .checked_mul(sy)
        .and_then(|n| n.checked_mul(2))
        .ok_or(LiquifyError::Malformed("size product overflows"))?;
    if disp.len() != expected {
        return Err(LiquifyError::DispLength {
            expected,
            actual: disp.len(),
        });
    }
    let mut out = Vec::with_capacity(sx * sy);
    for pair in disp.chunks_exact(2) {
        let dx = pair[0]
            .as_f64()
            .ok_or(LiquifyError::Malformed("disp entry is not a number"))?;
        let dy = pair[1]
            .as_f64()
            .ok_or(LiquifyError::Malformed("disp entry is not a number"))?;
        if !dx.is_finite() || !dy.is_finite() {
            return Err(LiquifyError::NonFinite("disp"));
        }
        out.push([dx as f32, dy as f32]);
    }
    Ok(LiquifyField {
        pitch: pitch as i32,
        origin: [ox, oy],
        nodes_x: sx,
        nodes_y: sy,
        disp: out,
        spacing: [pitch as f64, pitch as f64],
    })
}

// ---------------------------------------------------------------------------
// Commit resample (§16.3 / §45.4: one full-res §18.2 bicubic pass).
// ---------------------------------------------------------------------------

/// The commit engine: `out(p) = resample_bicubic(src, p + D(p))` with
/// the §18.2 Catmull-Rom kernel, premultiplied, out-of-source reads
/// transparent (§16.3). `src` must cover the displaced reads the caller
/// wants honored (the §16.1 ROI contract — `appname-filters` computes
/// the apron from [`LiquifyField::max_disp`]).
pub fn commit_plane(src: &Plane, field: &LiquifyField, out_rect: IRect) -> Plane {
    let kernel = Kernel::CatmullRom; // §16.3/§45.4 commit sampling
    let support = kernel.support();
    let mut out = Plane::new(out_rect);
    for y in out_rect.min.y..out_rect.max.y {
        for x in out_rect.min.x..out_rect.max.x {
            // Module pin: field evaluated at the output pixel center.
            let pc = [f64::from(x) + 0.5, f64::from(y) + 0.5];
            let d = field.sample(pc[0], pc[1]);
            // Back to sample space (−0.5 center shift, warp.rs precedent).
            let sx = pc[0] + d[0] - 0.5;
            let sy = pc[1] + d[1] - 0.5;
            let (lo_x, hi_x) = ((sx - support).ceil() as i64, (sx + support).floor() as i64);
            let (lo_y, hi_y) = ((sy - support).ceil() as i64, (sy + support).floor() as i64);
            let mut acc = [0.0f64; 4];
            let mut wsum = 0.0f64;
            for ky in lo_y..=hi_y {
                let wy = kernel.weight(ky as f64 - sy);
                if wy == 0.0 {
                    continue;
                }
                for kx in lo_x..=hi_x {
                    let w = kernel.weight(kx as f64 - sx) * wy;
                    if w == 0.0 {
                        continue;
                    }
                    let v = src.get(kx as i32, ky as i32, EdgePolicy::Transparent);
                    for c in 0..4 {
                        acc[c] += f64::from(v[c]) * w;
                    }
                    wsum += w;
                }
            }
            if wsum != 0.0 {
                let iw = 1.0 / wsum;
                out.set(
                    x,
                    y,
                    [
                        (acc[0] * iw) as f32,
                        (acc[1] * iw) as f32,
                        (acc[2] * iw) as f32,
                        (acc[3] * iw) as f32,
                    ],
                );
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use appname_core::IRect;

    fn canvas() -> IRect {
        IRect::new(0, 0, 64, 64)
    }

    fn field16() -> LiquifyField {
        LiquifyField::new(canvas(), 16).unwrap()
    }

    fn checker_plane(rect: IRect) -> Plane {
        let mut p = Plane::new(rect);
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                let v = if (x / 4 + y / 4) % 2 == 0 { 1.0 } else { 0.25 };
                p.set(x, y, [v, v * 0.5, 1.0 - v, 1.0]);
            }
        }
        p
    }

    #[test]
    fn node_counts_follow_the_45_4_rule() {
        let f = field16();
        // 64 px extent at pitch 16: ⌈64/16⌉ + 1 = 5 nodes per axis.
        assert_eq!((f.nodes_x, f.nodes_y), (5, 5));
        let f4 = LiquifyField::new(IRect::new(0, 0, 251, 188), 4).unwrap();
        assert_eq!((f4.nodes_x, f4.nodes_y), (64, 48));
        assert_eq!(
            LiquifyField::new(canvas(), 5),
            Err(LiquifyError::PitchInvalid(5))
        );
        assert_eq!(
            LiquifyField::new(IRect::EMPTY, 16),
            Err(LiquifyError::EmptyBounds)
        );
    }

    #[test]
    fn falloff_matches_the_16_3_base_at_density_half() {
        // density 0.5 ⇒ exponent 1 ⇒ §16.3's exact f = (1 − (d/R)²)².
        let r = 10.0;
        for d in [0.0, 2.5, 5.0, 9.9] {
            let t = d / r;
            let expect = (1.0 - t * t) * (1.0 - t * t);
            assert!((falloff(d, r, 0.5) - expect).abs() < 1e-12);
        }
        assert_eq!(falloff(10.0, 10.0, 0.5), 0.0);
        assert_eq!(falloff(11.0, 10.0, 0.5), 0.0);
        // density 1 ⇒ flat-hard inside; density 0 ⇒ f².
        assert_eq!(falloff(9.9, 10.0, 1.0), 1.0);
        let f = falloff(5.0, 10.0, 0.5);
        assert!((falloff(5.0, 10.0, 0.0) - f * f).abs() < 1e-12);
    }

    #[test]
    fn forward_warp_updates_against_the_16_3_formula() {
        let mut f = field16();
        let opts = BrushOptions {
            radius: 20.0,
            strength: 1.0,
            density: 0.5,
            alt: false,
        };
        // Cursor exactly on node (2, 2) = (32, 32); Δc = (4, 0).
        apply_brush_event(
            &mut f,
            LiquifyBrush::ForwardWarp,
            &opts,
            BrushEvent {
                cursor: [32.0, 32.0],
                delta: [4.0, 0.0],
            },
            canvas(),
        );
        // At the node itself d = 0, f = 1: D = −s·f·Δc = (−4, 0).
        let d = f.disp[2 * f.nodes_x + 2];
        assert_eq!(d, [-4.0, 0.0]);
        // A node 16 px away: f = (1 − (16/20)²)² = 0.1296.
        let d = f.disp[2 * f.nodes_x + 3];
        let expect = -4.0 * 0.1296;
        assert!((f64::from(d[0]) - expect).abs() < 1e-6, "{d:?}");
        assert_eq!(d[1], 0.0);
    }

    #[test]
    fn twirl_pucker_bloat_push_follow_their_formulas_and_alt_mapping() {
        let opts = BrushOptions {
            radius: 24.0,
            strength: 1.0,
            density: 0.5,
            alt: false,
        };
        let alt = BrushOptions { alt: true, ..opts };
        let ev = |c: [f64; 2], d: [f64; 2]| BrushEvent {
            cursor: c,
            delta: d,
        };
        // Probe node (3, 2) at (48, 32) with cursor c = (32, 32):
        // x − c = (16, 0), d = 16, f = (1 − (16/24)²)² ≈ 0.3086.
        let f_val = falloff(16.0, 24.0, 0.5);
        let idx = |f: &LiquifyField| f.disp[2 * f.nodes_x + 3];

        // Twirl clockwise: += s·f·ω·perp(x − c) = s·f·0.05·(0, 16).
        let mut fw = field16();
        apply_brush_event(
            &mut fw,
            LiquifyBrush::Twirl,
            &opts,
            ev([32.0, 32.0], [0.0, 0.0]),
            canvas(),
        );
        let d = idx(&fw);
        assert!((f64::from(d[1]) - f_val * 0.05 * 16.0).abs() < 1e-6);
        assert_eq!(d[0], 0.0);
        // Alt: counterclockwise (sign flip).
        let mut fa = field16();
        apply_brush_event(
            &mut fa,
            LiquifyBrush::Twirl,
            &alt,
            ev([32.0, 32.0], [0.0, 0.0]),
            canvas(),
        );
        assert_eq!(idx(&fa)[1], -d[1]);

        // Pucker: += s·f·κ·(x − c); Bloat is the sign mirror; Alt swaps.
        let mut fp = field16();
        apply_brush_event(
            &mut fp,
            LiquifyBrush::Pucker,
            &opts,
            ev([32.0, 32.0], [0.0, 0.0]),
            canvas(),
        );
        let dp = idx(&fp);
        assert!((f64::from(dp[0]) - f_val * 0.05 * 16.0).abs() < 1e-6);
        let mut fb = field16();
        apply_brush_event(
            &mut fb,
            LiquifyBrush::Bloat,
            &opts,
            ev([32.0, 32.0], [0.0, 0.0]),
            canvas(),
        );
        assert_eq!(idx(&fb)[0], -dp[0]);
        let mut fpa = field16();
        apply_brush_event(
            &mut fpa,
            LiquifyBrush::Pucker,
            &alt,
            ev([32.0, 32.0], [0.0, 0.0]),
            canvas(),
        );
        assert_eq!(idx(&fpa)[0], -dp[0], "Alt-Pucker acts as Bloat");

        // Push left: drag straight up (0, −2) pushes content left:
        // perp(Δĉ) = (1, 0)·… — §16.3's y-down convention gives
        // perp((0,−1)) = (1, 0)?  perp(v) = (−v.y, v.x) = (1, 0). The
        // §16.3 comment says "drag up pushes content left"; content
        // moves opposite the sample shift, so D gains +x. Formula
        // transcribed, direction asserted as the formula states.
        let mut fl = field16();
        apply_brush_event(
            &mut fl,
            LiquifyBrush::PushLeft,
            &opts,
            ev([32.0, 32.0], [0.0, -2.0]),
            canvas(),
        );
        let dl = idx(&fl);
        assert!((f64::from(dl[0]) - f_val * 2.0 * 1.0).abs() < 1e-6);
        assert_eq!(dl[1], 0.0);
        let mut flr = field16();
        apply_brush_event(
            &mut flr,
            LiquifyBrush::PushLeft,
            &alt,
            ev([32.0, 32.0], [0.0, -2.0]),
            canvas(),
        );
        assert_eq!(idx(&flr)[0], -dl[0], "Alt = push right");
    }

    #[test]
    fn reconstruct_relaxes_toward_identity_only_under_the_brush() {
        let mut f = field16();
        f.disp[2 * f.nodes_x + 2] = [8.0, -6.0];
        f.disp[0] = [3.0, 3.0]; // far node (corner) — outside radius
        let opts = BrushOptions {
            radius: 20.0,
            strength: 1.0,
            density: 0.5,
            alt: false,
        };
        for _ in 0..200 {
            apply_brush_event(
                &mut f,
                LiquifyBrush::Reconstruct,
                &opts,
                BrushEvent {
                    cursor: [32.0, 32.0],
                    delta: [0.0, 0.0],
                },
                canvas(),
            );
        }
        let d = f.disp[2 * f.nodes_x + 2];
        assert!(
            d[0].abs() < 1e-6 && d[1].abs() < 1e-6,
            "scrubbed node relaxes to identity: {d:?}"
        );
        assert_eq!(f.disp[0], [3.0, 3.0], "untouched regions keep their warp");
    }

    #[test]
    fn canvas_boundary_nodes_are_pinned() {
        let mut f = field16();
        let opts = BrushOptions {
            radius: 2000.0,
            strength: 1.0,
            density: 1.0, // flat: every unpinned node moves identically
            alt: false,
        };
        apply_brush_event(
            &mut f,
            LiquifyBrush::ForwardWarp,
            &opts,
            BrushEvent {
                cursor: [32.0, 32.0],
                delta: [5.0, 0.0],
            },
            canvas(),
        );
        // Node (0, j) sits at x = 0 = canvas.min.x → pinned; the corner
        // (4, 4) sits at (64, 64) = canvas.max → pinned.
        for j in 0..f.nodes_y {
            assert_eq!(f.disp[j * f.nodes_x], [0.0, 0.0], "left column pinned");
            assert_eq!(
                f.disp[j * f.nodes_x + f.nodes_x - 1],
                [0.0, 0.0],
                "right column pinned"
            );
        }
        // A strictly interior node moved.
        assert_eq!(f.disp[2 * f.nodes_x + 2], [-5.0, 0.0]);
    }

    #[test]
    fn field_sample_is_bilinear_and_zero_outside() {
        let mut f = field16();
        f.disp[2 * f.nodes_x + 2] = [8.0, 4.0]; // node (2,2) at (32,32)
        assert_eq!(f.sample(32.0, 32.0), [8.0, 4.0]);
        // Halfway to node (3,2): bilinear = half.
        let d = f.sample(40.0, 32.0);
        assert!((d[0] - 4.0).abs() < 1e-9 && (d[1] - 2.0).abs() < 1e-9);
        // Outside the node extent: zero (module pin).
        assert_eq!(f.sample(-1.0, 32.0), [0.0, 0.0]);
        assert_eq!(f.sample(65.0, 65.0), [0.0, 0.0]);
    }

    #[test]
    fn bmesh_round_trip_is_equal_and_matches_the_16_3_shape() {
        let mut f = field16();
        f.disp[3] = [-3.25, 1.5];
        let text = to_bmesh(&f);
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["schema"], BMESH_SCHEMA);
        assert_eq!(v["pitch"], 16);
        assert_eq!(v["size"], json!([5, 5]));
        let back = parse_bmesh(&text).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn bmesh_reject_matrix_is_typed_and_total() {
        // §45.4 validation rows + the fuzz-shape extras. Every reject is
        // a typed error; none panics.
        let ok = to_bmesh(&field16());
        assert!(parse_bmesh(&ok).is_ok());
        let cases: Vec<(Value, &str)> = vec![
            (
                json!({"schema": "appname.mesh.v2", "pitch": 16, "origin": [0,0], "size": [1,1], "disp": [0.0, 0.0]}),
                "v2 schema id rejects",
            ),
            (
                json!({"pitch": 16, "origin": [0,0], "size": [1,1], "disp": [0.0, 0.0]}),
                "schema missing",
            ),
            (
                json!({"schema": "appname.mesh.v1", "pitch": 5, "origin": [0,0], "size": [1,1], "disp": [0.0, 0.0]}),
                "pitch outside the set",
            ),
            (
                json!({"schema": "appname.mesh.v1", "pitch": 16, "origin": [0,0], "size": [2,2], "disp": [0.0, 0.0]}),
                "disp length mismatch",
            ),
            (
                json!({"schema": "appname.mesh.v1", "pitch": 16, "origin": [0,0], "size": [0,1], "disp": []}),
                "zero size axis",
            ),
            (
                // serde_json refuses out-of-range numbers at parse, so a
                // textual 1e999 rejects as malformed JSON; a programmatic
                // non-finite Value (json! of INFINITY serializes as null)
                // rejects on the number check. Both paths covered below.
                json!({"schema": "appname.mesh.v1", "pitch": 16, "origin": [0,0], "size": [1,1], "disp": [0.0, f64::INFINITY]}),
                "non-finite disp (null-encoded infinity)",
            ),
            (
                json!({"schema": "appname.mesh.v1", "pitch": 16, "origin": [0,0], "size": [1,1], "disp": [0.0, 0.0], "freeze": []}),
                "unknown key (v2 leak guard)",
            ),
            (
                json!({"schema": "appname.mesh.v1", "pitch": 16, "origin": [0.5, 0], "size": [1,1], "disp": [0.0, 0.0]}),
                "fractional origin",
            ),
            (
                json!({"schema": "appname.mesh.v1", "pitch": 16, "origin": [0,0], "size": [4611686018427387904i64, 4611686018427387904i64], "disp": []}),
                "size product overflow",
            ),
            (json!([1, 2, 3]), "non-object top level"),
        ];
        for (v, why) in cases {
            assert!(parse_bmesh(&v.to_string()).is_err(), "{why}");
        }
        assert!(parse_bmesh("not json").is_err());
        // Textual out-of-range float: serde_json refuses the number at
        // parse; the reject is typed either way (fuzz-shape guarantee).
        assert!(parse_bmesh(
            r#"{"schema": "appname.mesh.v1", "pitch": 16, "origin": [0,0], "size": [1,1], "disp": [0.0, 1e999]}"#
        )
        .is_err());
        assert!(parse_bmesh("").is_err());
    }

    #[test]
    fn fit_to_scales_positions_not_displacements() {
        let mut f = field16();
        f.disp[2 * f.nodes_x + 2] = [8.0, 4.0];
        let fitted = f.fit_to(IRect::new(0, 0, 128, 32)).unwrap();
        // Displacement VALUES unchanged.
        assert_eq!(fitted.disp, f.disp);
        // Node (2,2) now sits at (64, 16): positions scaled 2× / 0.5×.
        assert_eq!(fitted.node_pos(2, 2), [64.0, 16.0]);
        // The displacement sampled AT the scaled position is the node's.
        assert_eq!(fitted.sample(64.0, 16.0), [8.0, 4.0]);
        // Serialization still writes the canonical pitch (module pin).
        let v: Value = serde_json::from_str(&to_bmesh(&fitted)).unwrap();
        assert_eq!(v["pitch"], 16);
    }

    #[test]
    fn identity_field_commits_bit_identically() {
        // §45.8 prop_liquify_identity at the engine level.
        let rect = canvas();
        let src = checker_plane(rect);
        let out = commit_plane(&src, &field16(), rect);
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                assert_eq!(
                    out.get(x, y, EdgePolicy::Transparent),
                    src.get(x, y, EdgePolicy::Transparent),
                    "({x},{y})"
                );
            }
        }
    }

    #[test]
    fn integer_translation_field_shifts_content_exactly() {
        // D = (+8, 0) everywhere (backward map): out(p) = src(p + 8) —
        // integer taps stay exact through the CR kernel.
        let rect = canvas();
        let src = checker_plane(rect);
        let mut f = field16();
        for d in &mut f.disp {
            *d = [8.0, 0.0];
        }
        let out = commit_plane(&src, &f, rect);
        for y in 8..56 {
            for x in 8..48 {
                assert_eq!(
                    out.get(x, y, EdgePolicy::Transparent),
                    src.get(x + 8, y, EdgePolicy::Transparent),
                    "({x},{y})"
                );
            }
        }
    }

    #[test]
    fn out_of_source_reads_are_transparent_and_commit_is_deterministic() {
        let rect = canvas();
        let src = checker_plane(rect);
        let mut f = field16();
        for d in &mut f.disp {
            *d = [-100.0, 0.0]; // reads far left of the source
        }
        let out = commit_plane(&src, &f, rect);
        // Interior pixels (unpinned nodes) read fully outside → (0,0,0,0).
        assert_eq!(out.get(31, 31, EdgePolicy::Transparent), [0.0; 4]);
        // Determinism: same inputs, same bytes.
        let out2 = commit_plane(&src, &f, rect);
        assert_eq!(out, out2);
    }
}
