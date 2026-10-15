// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §22 FILL-CONTENT param registry — the `solid` / `gradient` / `pattern`
//! / `none` payload a FILL LAYER carries.
//!
//! Spec: `spec/schemas/document.v1.json` `$defs/fill` (the frozen v5.5
//! shape; the command contracts spell the same object `$defs/fillContent`
//! and already `$ref` it from `text.add`, `text.edit`, `text.set_style`,
//! `vector.shape_add` and `vector.shape_set_style`), `layer.add.yaml`
//! (`kind: fill`, `kindParams` = "open per-kind payload, re-validated
//! recursively at stage 3 against the kind's registry (§40.3): fill
//! content §22"), ENGINEERING-SPEC §22.
//!
//! # The contract letter
//!
//! `$defs/fill` is a four-arm `oneOf` discriminated on `type`:
//!
//! | `type`     | required     | optional                          |
//! |------------|--------------|-----------------------------------|
//! | `solid`    | `color`      | —                                 |
//! | `gradient` | `gradient`   | `angleDeg`=90, `scale`=1, `reverse`=false |
//! | `pattern`  | `patternRef` | `scale`=1, `phase`                |
//! | `none`     | —            | —                                 |
//!
//! All four are ADMITTED BY THE FROZEN CONTRACT; none of this is invented.
//! The gradient arm is the only place in v5.5 that carries gradient
//! GEOMETRY (`angleDeg`/`scale`): the `fill` COMMAND's gradient payload
//! carries none, and `fill.yaml` is `additionalProperties: false`, so the
//! two questions "may a fill LAYER hold gradient content?" and "may the
//! on-canvas gradient TOOL commit a drag?" have different answers — YES
//! and NO respectively. See [`crate::engine::gradient`]'s scope header
//! (O-INT split ruling `f429bc5`).
//!
//! # Two spellings of "a pattern", FLAGGED
//!
//! The same concept has two incompatible wire forms in the frozen spec:
//! the fill-LAYER arm here takes `patternRef`, a STRING documented as
//! `assets/{assetHash}` (§7.5, §22.5), while `fill.yaml`'s COMMAND
//! payload takes `patternId`, a `format: uuid`. A content-addressed asset
//! path and a UUID are not the same identifier space. This registry
//! implements the letter of ITS OWN arm (`patternRef`, string) and does
//! not reconcile the two — that is a v5.6 drift-gate item.
//!
//! # Modeling choice the contract does not pin (FLAGGED for O-INT)
//!
//! `layer.add{kind:"fill"}`'s `kindParams` is read as BEING the
//! `$defs/fill` object (`{type: "solid", color: {…}}`), not as a wrapper
//! around it (`{fill: {…}}`). `layer.add.yaml` says only "fill content
//! §22"; the document schema names the LAYER NODE's field `fill`, which is
//! where the wrapper reading would come from. One reading is implemented,
//! deliberately, rather than accepting both and blessing an ambiguity.
//!
//! # Divergence this module INHERITS, flagged not hidden
//!
//! Non-finite numerics are REFUSED here (`error.schema.out_of_range`),
//! not clamped as §12680 [NORMATIVE] / §0.3 direct. The reasoning, the
//! on-main precedent it follows, and the O-INT ruling it wants are
//! written out once in [`crate::engine::gradient`]'s module header.
//!
//! # Resolution is NOT this module's job
//!
//! A `patternRef` is decoded and range-checked here but NOT resolved
//! against the asset store; per `fill.yaml`, a pattern asset that fails to
//! resolve is `error.command.bad_target` at stage 4, which is the command
//! layer's call, not the registry's.

use crate::engine::error::{EngineError, ErrorCode};
use crate::engine::gradient::{Gradient, GradientKind, GradientSpace, GradientStop};
use crate::io::srgb::srgb_decode;
use serde_json::{Map, Value};

/// A decoded §22 fill content — the payload of a fill layer, and the same
/// object a text run or a shape fills with.
#[derive(Clone, Debug, PartialEq)]
pub enum FillContent {
    Solid {
        /// Straight working-space LINEAR RGB.
        color: [f64; 3],
        alpha: f64,
    },
    Gradient {
        gradient: Gradient,
        angle_deg: f64,
        scale: f64,
        reverse: bool,
    },
    Pattern {
        /// `assets/{assetHash}` (§7.5, §22.5) — see the module header on
        /// the `patternRef` / `patternId` split.
        pattern_ref: String,
        scale: f64,
        phase: [f64; 2],
    },
    None,
}

impl FillContent {
    /// The wire `type` discriminant this content decoded from.
    pub fn wire_type(&self) -> &'static str {
        match self {
            FillContent::Solid { .. } => "solid",
            FillContent::Gradient { .. } => "gradient",
            FillContent::Pattern { .. } => "pattern",
            FillContent::None => "none",
        }
    }
}

/// §22.1 colour object → straight working-space linear RGB **plus alpha**.
///
/// Mirrors the posture of `kinds.rs`'s `decode_color_object` (explicit
/// profiles refuse before the CMM; cmyk/lab refuse as not-implemented)
/// but also yields the ALPHA component, which the gradient stop ramp needs
/// and the RGB-only helper cannot express. A shared home comes with the §5
/// colour wave — the same note the existing helper carries.
fn decode_color_rgba(
    color: &Map<String, Value>,
    field: &str,
) -> Result<([f64; 3], f64), EngineError> {
    if color.get("profileId").is_some() {
        return Err(EngineError::with_field(
            ErrorCode::ProfileSpaceMismatch,
            format!("{field}.profileId"),
            "explicit colour profiles cannot be verified before the CMM (§5.5); omit profileId",
        ));
    }
    let comps = color
        .get("comps")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::missing(&format!("{field}.comps")))?;
    if comps.len() != 4 || !comps.iter().all(|c| c.as_f64().is_some_and(f64::is_finite)) {
        return Err(EngineError::range(
            &format!("{field}.comps"),
            "`comps` must be 4 finite numbers (§22.1)",
        ));
    }
    let c = |i: usize| comps[i].as_f64().unwrap_or(0.0).clamp(0.0, 1.0);
    match color.get("model").and_then(Value::as_str) {
        Some("rgb") => Ok(([srgb(c(0)), srgb(c(1)), srgb(c(2))], 1.0)),
        Some("rgba") => Ok(([srgb(c(0)), srgb(c(1)), srgb(c(2))], c(3))),
        Some("gray") => {
            let v = srgb(c(0));
            Ok(([v, v, v], 1.0))
        }
        Some("grayA") => {
            let v = srgb(c(0));
            Ok(([v, v, v], c(1)))
        }
        Some(other @ ("cmyk" | "cmykA" | "lab" | "labA")) => Err(EngineError::new(
            ErrorCode::NotImplemented,
            format!("colour model \"{other}\" needs the §5/§6 conversion pipeline"),
        )),
        Some(other) => Err(EngineError::range(
            &format!("{field}.model"),
            format!("unknown colour model \"{other}\" (§7.3 enum)"),
        )),
        None => Err(EngineError::missing(&format!("{field}.model"))),
    }
}

fn srgb(v: f64) -> f64 {
    f64::from(srgb_decode(v as f32))
}

/// Read an optional number, refusing a present-but-wrong-typed or
/// non-finite value rather than silently falling back to the default —
/// a typo'd `scale` must not read as 1.
fn opt_num(
    m: &Map<String, Value>,
    key: &str,
    field: &str,
    default: f64,
) -> Result<f64, EngineError> {
    match m.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| {
            EngineError::range(&format!("{field}.{key}"), "must be a finite number")
        }),
    }
}

fn opt_bool(
    m: &Map<String, Value>,
    key: &str,
    field: &str,
    default: bool,
) -> Result<bool, EngineError> {
    match m.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_bool()
            .ok_or_else(|| EngineError::range(&format!("{field}.{key}"), "must be a boolean")),
    }
}

/// Decode a §22.4 `gradient` object (the `$defs/gradient` shape).
pub fn decode_gradient(v: &Value, field: &str) -> Result<Gradient, EngineError> {
    let m = v
        .as_object()
        .ok_or_else(|| EngineError::range(field, "`gradient` must be an object (§22.4)"))?;
    let kind_s = m
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::missing(&format!("{field}.kind")))?;
    let kind = GradientKind::from_wire(kind_s).ok_or_else(|| {
        EngineError::range(
            &format!("{field}.kind"),
            format!("unknown gradient kind \"{kind_s}\" (§22.4 enum)"),
        )
    })?;
    let space = match m.get("space") {
        None | Some(Value::Null) => GradientSpace::default(),
        Some(v) => {
            let s = v
                .as_str()
                .ok_or_else(|| EngineError::range(&format!("{field}.space"), "must be a string"))?;
            GradientSpace::from_wire(s).ok_or_else(|| {
                EngineError::range(
                    &format!("{field}.space"),
                    format!("unknown gradient space \"{s}\" (§22.4 enum)"),
                )
            })?
        }
    };
    let dither = opt_bool(m, "dither", field, true)?;
    let stops_v = m
        .get("stops")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::missing(&format!("{field}.stops")))?;
    let mut stops = Vec::with_capacity(stops_v.len());
    for (i, sv) in stops_v.iter().enumerate() {
        let sf = format!("{field}.stops/{i}");
        let sm = sv
            .as_object()
            .ok_or_else(|| EngineError::range(&sf, "a gradient stop must be an object"))?;
        let pos = sm
            .get("pos")
            .and_then(Value::as_f64)
            .ok_or_else(|| EngineError::missing(&format!("{sf}.pos")))?;
        let color_m = sm
            .get("color")
            .and_then(Value::as_object)
            .ok_or_else(|| EngineError::missing(&format!("{sf}.color")))?;
        let (color, color_alpha) = decode_color_rgba(color_m, &format!("{sf}.color"))?;
        let opacity = opt_num(sm, "opacity", &sf, 1.0)?;
        if !(0.0..=1.0).contains(&opacity) {
            return Err(EngineError::range(
                &format!("{sf}.opacity"),
                "stop opacity must be in 0..=1 (§22.4)",
            ));
        }
        let midpoint = opt_num(sm, "midpoint", &sf, 0.5)?;
        stops.push(GradientStop {
            pos,
            color,
            // The colour's own alpha and the stop's opacity MULTIPLY: the
            // colour says how opaque the ink is, the stop says how much of
            // it lands at this position. Neither overrides the other.
            alpha: color_alpha * opacity,
            midpoint,
        });
    }
    // Gradient::new establishes the >=2 / sorted / in-range invariants and
    // returns the typed range errors; it is the single validation gate so
    // this decoder and any other caller cannot drift apart.
    Gradient::new(stops, kind, space, dither)
}

/// Decode a `$defs/fill` (a.k.a. `$defs/fillContent`) object.
///
/// Known fields are validated strictly (wrong type / bad enum / out of
/// range → typed refusal against the wire field); unknown fields pass
/// through untouched, matching the schema (the arms do not set
/// `additionalProperties: false`) and the §7.7 forward-compatibility rule.
pub fn decode_fill_content(v: &Value, field: &str) -> Result<FillContent, EngineError> {
    let m = v
        .as_object()
        .ok_or_else(|| EngineError::range(field, "fill content must be an object (§22)"))?;
    let ty = m
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::missing(&format!("{field}.type")))?;
    match ty {
        "none" => Ok(FillContent::None),
        "solid" => {
            let c = m
                .get("color")
                .and_then(Value::as_object)
                .ok_or_else(|| EngineError::missing(&format!("{field}.color")))?;
            let (color, alpha) = decode_color_rgba(c, &format!("{field}.color"))?;
            Ok(FillContent::Solid { color, alpha })
        }
        "gradient" => {
            let g = m
                .get("gradient")
                .ok_or_else(|| EngineError::missing(&format!("{field}.gradient")))?;
            let gradient = decode_gradient(g, &format!("{field}.gradient"))?;
            let angle_deg = opt_num(m, "angleDeg", field, 90.0)?;
            let scale = opt_num(m, "scale", field, 1.0)?;
            if !(scale > 0.0) {
                return Err(EngineError::range(
                    &format!("{field}.scale"),
                    "scale must be > 0 (§7.3 exclusiveMinimum)",
                ));
            }
            let reverse = opt_bool(m, "reverse", field, false)?;
            Ok(FillContent::Gradient {
                gradient,
                angle_deg,
                scale,
                reverse,
            })
        }
        "pattern" => {
            let pattern_ref = m
                .get("patternRef")
                .and_then(Value::as_str)
                .ok_or_else(|| EngineError::missing(&format!("{field}.patternRef")))?;
            if pattern_ref.is_empty() {
                return Err(EngineError::range(
                    &format!("{field}.patternRef"),
                    "patternRef must be a non-empty `assets/{assetHash}` reference (§7.5, §22.5)",
                ));
            }
            let scale = opt_num(m, "scale", field, 1.0)?;
            if !(scale > 0.0) {
                return Err(EngineError::range(
                    &format!("{field}.scale"),
                    "scale must be > 0 (§7.3 exclusiveMinimum)",
                ));
            }
            let phase = match m.get("phase") {
                None | Some(Value::Null) => [0.0, 0.0],
                Some(v) => {
                    let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| {
                        EngineError::range(&format!("{field}.phase"), "phase must be [x, y]")
                    })?;
                    let n = |i: usize| {
                        a[i].as_f64().filter(|f| f.is_finite()).ok_or_else(|| {
                            EngineError::range(
                                &format!("{field}.phase/{i}"),
                                "phase components must be finite numbers",
                            )
                        })
                    };
                    [n(0)?, n(1)?]
                }
            };
            Ok(FillContent::Pattern {
                pattern_ref: pattern_ref.to_string(),
                scale,
                phase,
            })
        }
        other => Err(EngineError::range(
            &format!("{field}.type"),
            format!("unknown fill type \"{other}\" (§7.3 FillContent enum)"),
        )),
    }
}

// ---------------------------------------------------------------------------
// FillContent → PIXELS (the composite leg)
// ---------------------------------------------------------------------------

/// Rasterize §22 fill content over `box_rect` into a PREMULTIPLIED-linear
/// page — the `FillContent` → pixels leg, and the ONE place it happens.
///
/// Both composite doors reach this through
/// [`crate::io::png_io::resolve_parametric_page`] (the reference export's
/// `parametric_sources` walk and `appname-render`'s compositor), so the
/// two doors cannot disagree about what a fill layer looks like: there is
/// only one function. That is the same one-math-library posture the shape
/// and text legs already ride, and it is what makes the QA-F8 serve↔refuse
/// question answerable in one place instead of two.
///
/// # Sampling
///
/// The gradient field is continuous; a pixel is an area. Each pixel is
/// sampled at its CENTRE, `(x + ½, y + ½)`, which is the convention the
/// rest of the engine's continuous→discrete seams use and the only choice
/// under which the box's two edges are equidistant from the first and last
/// sample. Combined with [`crate::engine::gradient::GradientGeometry::
/// from_layer_box`] sizing the axis to span the box exactly, the extreme
/// pixel CENTRES land at `t = ½/w` and `t = 1 − ½/w` rather than at 0 and
/// 1 — correct, and NOT a bug: the ramp's endpoints sit on the box's
/// boundary, which is half a pixel outside the outermost centres.
///
/// # What this does NOT do, and why — DITHER
///
/// §22.4's dithering clause is [NORMATIVE] and is explicit about WHERE it
/// applies: "with mask value `m ∈ [0,1)`, each channel adds `(m − 0.5)/255`
/// … immediately **before** quantization. Applied at fill time and at
/// export (§29.1); **never stored in float buffers**." This function
/// produces an f32 premultiplied-linear page — a float buffer — so
/// applying the blue-noise mask HERE would violate the clause that
/// specifies it, and would bake noise into a value that is then blended,
/// masked and re-quantized downstream (dithering the wrong quantity, at
/// the wrong precision, more than once).
///
/// The `dither` flag is therefore decoded, carried on the [`Gradient`],
/// and deliberately not consumed here. **FLAGGED, not silently dropped:**
/// the engine has the mask (`appname-render`'s `BN64`, the §4.5
/// void-and-cluster table) and has the demotion seam
/// (`appname-color`'s `demote_f32_dithered`), but there is no plumbing
/// that carries a PER-GRADIENT `dither: bool` down to the whole-composite
/// quantization step where §22.4 says it must act — quantization sees a
/// flattened composite, not the gradient that contributed a pixel. Wiring
/// that is a real row (it decides whether the flag is per-gradient or
/// promotes to a document/export-level setting, which is a contract
/// question), and it is NOT this leg. Consequence to state plainly: §22.4's
/// "[NORMATIVE] Gradients are bandless at 8-bit with dithering on (tested)"
/// is NOT yet satisfied end-to-end, and this slice does not claim it is.
///
/// # Refusals (typed, never omitted — QA-F8)
///
/// * `pattern` — see the arm's own note below.
/// * `gradient` with `kind: "noise"` — no parameters exist to synthesize
///   it (the `$defs/gradient` payload carries no seed/roughness/ranges);
///   the refusal comes from the single site in
///   [`crate::engine::gradient`], probed once up front.
///
/// On that probe, precisely: it hoists the kind check OUT of the per-pixel
/// loop, so an unservable kind costs O(1) and a typed refusal instead of a
/// full-canvas allocation walked to no purpose. It is NOT what protects
/// the degenerate-box case — `GradientGeometry::from_layer_box` refuses an
/// empty box first, with `out_of_range`, so the loop can never be entered
/// zero times behind a kind that should have refused. (An earlier draft of
/// this comment claimed the probe was that protection; the test written to
/// pin it failed and disproved it. The ordering is pinned by
/// `noise_never_yields_a_page_whatever_the_box`.)
pub fn render_fill_page(
    content: &FillContent,
    box_rect: crate::geom::IRect,
) -> Result<crate::engine::text_ops::TextPageData, EngineError> {
    let (w, h) = (box_rect.width().max(0), box_rect.height().max(0));
    let n = (w as usize) * (h as usize);

    // Premultiply a STRAIGHT linear RGBA sample into the page's storage
    // form. The page contract is premultiplied linear f32 (§4.4 works on
    // premultiplied values), and `Gradient::eval` / the solid arm both
    // yield STRAIGHT — so this is the single conversion point.
    let premul = |c: [f64; 4]| -> [f32; 4] {
        let a = c[3];
        [
            (c[0] * a) as f32,
            (c[1] * a) as f32,
            (c[2] * a) as f32,
            a as f32,
        ]
    };

    let pixels: Vec<[f32; 4]> = match content {
        // `none` is a SERVED arm, not a refusal: the contract admits it,
        // and it means "this layer contributes nothing". A fully
        // transparent page source-overs to identity (§4.4), so the
        // document composites exactly as if the layer were absent —
        // which is the letter of "none". Refusing it would wall a
        // perfectly valid document.
        FillContent::None => vec![[0.0; 4]; n],

        FillContent::Solid { color, alpha } => {
            vec![premul([color[0], color[1], color[2], *alpha]); n]
        }

        FillContent::Gradient {
            gradient,
            angle_deg,
            scale,
            reverse,
        } => {
            let geo = crate::engine::gradient::GradientGeometry::from_layer_box(
                [f64::from(box_rect.min.x), f64::from(box_rect.min.y)],
                [f64::from(box_rect.max.x), f64::from(box_rect.max.y)],
                *angle_deg,
                *scale,
            )?;
            // Probe the kind ONCE before the loop, so an unservable kind
            // (`noise`) costs O(1) and a typed refusal instead of a
            // full-canvas allocation walked to no purpose, and so the
            // refusal cannot depend on iteration order.
            //
            // It is NOT what protects the degenerate-box case —
            // `from_layer_box` above already refused an empty box. See
            // this function's doc comment: an earlier draft claimed
            // otherwise and the test written to pin it disproved the
            // claim.
            geo.parameter_at(gradient.kind(), geo.start())?;

            let mut out = Vec::with_capacity(n);
            for y in box_rect.min.y..box_rect.max.y {
                for x in box_rect.min.x..box_rect.max.x {
                    let p = [f64::from(x) + 0.5, f64::from(y) + 0.5];
                    out.push(premul(geo.sample(gradient, p, *reverse)?));
                }
            }
            out
        }

        // §22.5 pattern content REFUSES TYPED, and this is a statement
        // about the engine's state, verified first-hand rather than
        // assumed:
        //
        //   * there is no `edit.define_pattern` command (§22.5's own
        //     definition door) anywhere in the tree;
        //   * there is no `appname.pattern.v1` asset schema — so nothing
        //     carries the `size` / `defaultTransform{offsetPct, scalePct,
        //     angle}` that §22.5's tiling transform is defined in terms of;
        //   * [`crate::engine::assets::AssetKind`] has no pattern arm, and
        //     asset ids are content-derived UUIDs while `patternRef` is a
        //     STRING (`assets/{assetHash}`) — the identifier spaces do not
        //     even meet (the `patternRef`/`patternId` split this module's
        //     header already flags as a v5.6 drift-gate item);
        //   * §22.5 sampling is defined as "filtered per §18.2" (bicubic
        //     Catmull-Rom magnified, Lanczos-3 over the pattern's own mip
        //     pyramid minified), and those kernels live in
        //     `appname-tools`, which core reaches only through an
        //     assembly-injected seam that does not exist for patterns.
        //
        // There is therefore no asset to resolve, no transform to resolve
        // it with, and no filter to sample it through. Serving this arm
        // would mean INVENTING all three — a contract, not an
        // implementation. It refuses typed, uniformly at both doors,
        // exactly as `noise` does, and never renders a wrong picture or
        // silently omits the layer.
        FillContent::Pattern { pattern_ref, .. } => {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                format!(
                    "fill content \"pattern\" (patternRef {pattern_ref:?}) cannot be \
                     composited: §22.5 pattern assets have no definition door \
                     (edit.define_pattern), no `appname.pattern.v1` schema and no \
                     AssetKind, so there is nothing to resolve the reference against, \
                     and §22.5 tiling samples through the §18.2 filters which core \
                     has no seam for — refused typed, never omitted (QA-F8); v5.6 \
                     fold-in row"
                ),
            ));
        }
    };

    debug_assert_eq!(pixels.len(), n, "page must cover its rect exactly");
    Ok(crate::engine::text_ops::TextPageData {
        rect: box_rect,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dec(v: Value) -> Result<FillContent, EngineError> {
        decode_fill_content(&v, "kindParams")
    }

    fn code(v: Value) -> String {
        dec(v).unwrap_err().code.as_str().to_string()
    }

    // -- the four contract arms ------------------------------------------

    #[test]
    fn all_four_frozen_arms_decode() {
        // The whole point of the registry: every arm the frozen v5.5
        // `$defs/fill` admits must round-trip to its typed form.
        assert_eq!(dec(json!({"type": "none"})).unwrap().wire_type(), "none");
        assert_eq!(
            dec(json!({"type": "solid", "color": {"model": "rgb", "comps": [1, 0, 0, 0]}}))
                .unwrap()
                .wire_type(),
            "solid"
        );
        assert_eq!(
            dec(json!({"type": "pattern", "patternRef": "assets/abc123"}))
                .unwrap()
                .wire_type(),
            "pattern"
        );
        assert_eq!(
            dec(json!({
                "type": "gradient",
                "gradient": {
                    "kind": "linear",
                    "stops": [
                        {"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]}},
                        {"pos": 1, "color": {"model": "rgb", "comps": [1, 1, 1, 0]}}
                    ]
                }
            }))
            .unwrap()
            .wire_type(),
            "gradient"
        );
    }

    #[test]
    fn solid_decodes_through_the_srgb_eotf_with_alpha() {
        // 1.0 stays 1.0 and 0.0 stays 0.0 through the EOTF, but the
        // MIDDLE must move — sRGB 0.5 is ~0.2140 in linear light. A
        // decoder that forgot to linearize would return 0.5.
        let f = dec(json!({
            "type": "solid",
            "color": {"model": "rgba", "comps": [0.5, 1.0, 0.0, 0.25]}
        }))
        .unwrap();
        let FillContent::Solid { color, alpha } = f else {
            panic!("expected solid")
        };
        assert!(
            (color[0] - 0.214_041).abs() < 1e-5,
            "sRGB 0.5 must linearize to ~0.214041, got {}",
            color[0]
        );
        assert_eq!(color[1], 1.0, "1.0 is a fixed point of the EOTF");
        assert_eq!(color[2], 0.0, "0.0 is a fixed point of the EOTF");
        assert_eq!(alpha, 0.25, "alpha is NOT gamma-encoded");
    }

    #[test]
    fn gray_models_broadcast_and_carry_alpha() {
        let FillContent::Solid { color, alpha } =
            dec(json!({"type": "solid", "color": {"model": "grayA", "comps": [1.0, 0.5, 0, 0]}}))
                .unwrap()
        else {
            panic!("expected solid")
        };
        assert_eq!(color, [1.0, 1.0, 1.0], "gray must broadcast to all three");
        assert_eq!(alpha, 0.5, "grayA's second comp is alpha");
    }

    // -- defaults ---------------------------------------------------------

    #[test]
    fn gradient_arm_defaults_match_the_contract() {
        let f = dec(json!({
            "type": "gradient",
            "gradient": {
                "kind": "radial",
                "stops": [
                    {"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]}},
                    {"pos": 1, "color": {"model": "rgb", "comps": [1, 1, 1, 0]}}
                ]
            }
        }))
        .unwrap();
        let FillContent::Gradient {
            gradient,
            angle_deg,
            scale,
            reverse,
        } = f
        else {
            panic!("expected gradient")
        };
        assert_eq!(angle_deg, 90.0, "$defs/fill angleDeg default is 90");
        assert_eq!(scale, 1.0, "$defs/fill scale default is 1");
        assert!(!reverse, "$defs/fill reverse default is false");
        assert_eq!(gradient.space(), GradientSpace::LinearRgb, "space default");
        assert!(gradient.dither(), "$defs/gradient dither default is true");
        assert_eq!(gradient.stops()[0].midpoint, 0.5, "stop midpoint default");
        assert_eq!(gradient.stops()[0].alpha, 1.0, "stop opacity default");
    }

    #[test]
    fn pattern_arm_defaults_match_the_contract() {
        let FillContent::Pattern {
            pattern_ref,
            scale,
            phase,
        } = dec(json!({"type": "pattern", "patternRef": "assets/deadbeef"})).unwrap()
        else {
            panic!("expected pattern")
        };
        assert_eq!(pattern_ref, "assets/deadbeef");
        assert_eq!(scale, 1.0);
        assert_eq!(phase, [0.0, 0.0]);
    }

    #[test]
    fn stop_opacity_multiplies_the_colour_alpha() {
        let f = dec(json!({
            "type": "gradient",
            "gradient": {
                "kind": "linear",
                "stops": [
                    {"pos": 0, "color": {"model": "rgba", "comps": [0, 0, 0, 0.5]}, "opacity": 0.5},
                    {"pos": 1, "color": {"model": "rgb", "comps": [1, 1, 1, 0]}}
                ]
            }
        }))
        .unwrap();
        let FillContent::Gradient { gradient, .. } = f else {
            panic!("expected gradient")
        };
        assert_eq!(
            gradient.stops()[0].alpha,
            0.25,
            "colour alpha 0.5 * stop opacity 0.5 must be 0.25, not either alone"
        );
    }

    // -- cross-field refusals (the §40.5 shape) ---------------------------

    #[test]
    fn each_arm_refuses_its_missing_payload() {
        assert_eq!(code(json!({"type": "solid"})), "error.schema.missing_field");
        assert_eq!(
            code(json!({"type": "gradient"})),
            "error.schema.missing_field"
        );
        assert_eq!(
            code(json!({"type": "pattern"})),
            "error.schema.missing_field"
        );
        assert_eq!(code(json!({})), "error.schema.missing_field");
    }

    #[test]
    fn unknown_fill_type_is_refused_typed() {
        assert_eq!(code(json!({"type": "plaid"})), "error.schema.out_of_range");
        assert_eq!(code(json!({"type": 7})), "error.schema.missing_field");
        assert_eq!(code(json!("solid")), "error.schema.out_of_range");
    }

    #[test]
    fn unknown_gradient_kind_and_space_are_refused() {
        let g = |k: Value, s: Value| {
            json!({
                "type": "gradient",
                "gradient": {
                    "kind": k, "space": s,
                    "stops": [
                        {"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]}},
                        {"pos": 1, "color": {"model": "rgb", "comps": [1, 1, 1, 0]}}
                    ]
                }
            })
        };
        assert_eq!(
            code(g(json!("spiral"), json!("linearRgb"))),
            "error.schema.out_of_range"
        );
        assert_eq!(
            code(g(json!("linear"), json!("hsv"))),
            "error.schema.out_of_range"
        );
        // ...and the frozen enums are ACCEPTED, in full.
        for k in [
            "linear",
            "radial",
            "angular",
            "reflected",
            "diamond",
            "noise",
        ] {
            assert!(
                dec(g(json!(k), json!("linearRgb"))).is_ok(),
                "kind {k} is frozen-legal"
            );
        }
        for s in ["linearRgb", "perceptual"] {
            assert!(
                dec(g(json!("linear"), json!(s))).is_ok(),
                "space {s} is frozen-legal"
            );
        }
    }

    #[test]
    fn noise_decodes_but_does_not_evaluate() {
        // `noise` is in the frozen enum, so the REGISTRY must accept it;
        // the refusal belongs at evaluation, where the missing parameters
        // actually bite. This pins that split.
        let f = dec(json!({
            "type": "gradient",
            "gradient": {
                "kind": "noise",
                "stops": [
                    {"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]}},
                    {"pos": 1, "color": {"model": "rgb", "comps": [1, 1, 1, 0]}}
                ]
            }
        }))
        .unwrap();
        let FillContent::Gradient { gradient, .. } = f else {
            panic!("expected gradient")
        };
        assert_eq!(gradient.kind(), GradientKind::Noise);
        let geo =
            crate::engine::gradient::GradientGeometry::from_drag([0.0, 0.0], [1.0, 0.0]).unwrap();
        assert_eq!(
            geo.sample(&gradient, [0.5, 0.0], false)
                .unwrap_err()
                .code
                .as_str(),
            "error.command.not_implemented"
        );
    }

    #[test]
    fn scale_must_be_strictly_positive_on_both_arms() {
        let grad = |s: Value| {
            json!({
                "type": "gradient", "scale": s,
                "gradient": {
                    "kind": "linear",
                    "stops": [
                        {"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]}},
                        {"pos": 1, "color": {"model": "rgb", "comps": [1, 1, 1, 0]}}
                    ]
                }
            })
        };
        assert_eq!(code(grad(json!(0))), "error.schema.out_of_range");
        assert_eq!(code(grad(json!(-2))), "error.schema.out_of_range");
        assert!(dec(grad(json!(0.001))).is_ok());
        let pat = |s: Value| json!({"type": "pattern", "patternRef": "assets/x", "scale": s});
        assert_eq!(code(pat(json!(0))), "error.schema.out_of_range");
        assert_eq!(code(pat(json!(-1))), "error.schema.out_of_range");
    }

    #[test]
    fn wrong_typed_optionals_refuse_instead_of_defaulting() {
        // A typo'd optional must NOT silently read as its default — that
        // is the difference between a caught error and a wrong picture.
        let g = |extra: Value| {
            let mut o = json!({
                "type": "gradient",
                "gradient": {
                    "kind": "linear",
                    "stops": [
                        {"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]}},
                        {"pos": 1, "color": {"model": "rgb", "comps": [1, 1, 1, 0]}}
                    ]
                }
            });
            let m = o.as_object_mut().unwrap();
            for (k, v) in extra.as_object().unwrap() {
                m.insert(k.clone(), v.clone());
            }
            o
        };
        assert_eq!(
            code(g(json!({"angleDeg": "90"}))),
            "error.schema.out_of_range"
        );
        assert_eq!(code(g(json!({"scale": "1"}))), "error.schema.out_of_range");
        assert_eq!(
            code(g(json!({"reverse": "false"}))),
            "error.schema.out_of_range"
        );
        // ...but an explicit null reads as ABSENT, so the default applies.
        let FillContent::Gradient { angle_deg, .. } = dec(g(json!({"angleDeg": null}))).unwrap()
        else {
            panic!("expected gradient")
        };
        assert_eq!(angle_deg, 90.0);
    }

    #[test]
    fn phase_shape_is_enforced() {
        let p = |ph: Value| json!({"type": "pattern", "patternRef": "assets/x", "phase": ph});
        assert_eq!(code(p(json!([1]))), "error.schema.out_of_range");
        assert_eq!(code(p(json!([1, 2, 3]))), "error.schema.out_of_range");
        assert_eq!(code(p(json!(["a", "b"]))), "error.schema.out_of_range");
        assert_eq!(code(p(json!(5))), "error.schema.out_of_range");
        let FillContent::Pattern { phase, .. } = dec(p(json!([-3.5, 7.0]))).unwrap() else {
            panic!("expected pattern")
        };
        assert_eq!(phase, [-3.5, 7.0]);
    }

    #[test]
    fn empty_pattern_ref_is_refused() {
        assert_eq!(
            code(json!({"type": "pattern", "patternRef": ""})),
            "error.schema.out_of_range"
        );
        assert_eq!(
            code(json!({"type": "pattern", "patternRef": 42})),
            "error.schema.missing_field"
        );
    }

    // -- colour posture ---------------------------------------------------

    #[test]
    fn explicit_profiles_and_uncorverted_models_refuse_as_they_do_elsewhere() {
        assert_eq!(
            code(json!({
                "type": "solid",
                "color": {"model": "rgb", "comps": [0, 0, 0, 0],
                          "profileId": "00000000-0008-4000-8000-000000000009"}
            })),
            "error.color.profile_space_mismatch"
        );
        assert_eq!(
            code(json!({"type": "solid", "color": {"model": "cmyk", "comps": [0, 0, 0, 1]}})),
            "error.command.not_implemented"
        );
        assert_eq!(
            code(json!({"type": "solid", "color": {"model": "chartreuse", "comps": [0, 0, 0, 1]}})),
            "error.schema.out_of_range"
        );
        assert_eq!(
            code(json!({"type": "solid", "color": {"comps": [0, 0, 0, 1]}})),
            "error.schema.missing_field"
        );
        assert_eq!(
            code(json!({"type": "solid", "color": {"model": "rgb", "comps": [0, 0, 1]}})),
            "error.schema.out_of_range"
        );
        assert_eq!(
            code(json!({"type": "solid", "color": {"model": "rgb", "comps": [0, 0, "x", 1]}})),
            "error.schema.out_of_range"
        );
    }

    #[test]
    fn unknown_fields_pass_through() {
        // §7.7 forward compatibility: the schema arms do not set
        // additionalProperties:false, so a field from a later spec
        // version must not break a v1 reader.
        assert!(dec(json!({"type": "none", "futureThing": 1})).is_ok());
        assert!(dec(json!({
            "type": "solid",
            "color": {"model": "rgb", "comps": [0, 0, 0, 0]},
            "v56Field": {"nested": true}
        }))
        .is_ok());
    }

    #[test]
    fn stops_below_two_refuse_through_the_single_gate() {
        assert_eq!(
            code(json!({
                "type": "gradient",
                "gradient": {
                    "kind": "linear",
                    "stops": [{"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]}}]
                }
            })),
            "error.schema.out_of_range"
        );
        assert_eq!(
            code(json!({"type": "gradient", "gradient": {"kind": "linear"}})),
            "error.schema.missing_field"
        );
    }

    // -- render_fill_page: FillContent -> PIXELS -------------------------

    use crate::geom::IRect;

    fn page(
        v: Value,
        w: i32,
        h: i32,
    ) -> Result<crate::engine::text_ops::TextPageData, EngineError> {
        render_fill_page(&dec(v).unwrap(), IRect::new(0, 0, w, h))
    }

    /// The page contract is PREMULTIPLIED linear, and the solid arm's job
    /// is to convert into it. Half-alpha red is the discriminating case:
    /// straight storage would leave 1.0 in the red channel, premultiplied
    /// leaves 0.5. A test at alpha 1.0 could not tell the two apart.
    #[test]
    fn solid_pages_are_premultiplied_over_the_whole_box() {
        let p = page(
            json!({"type": "solid", "color": {"model": "rgba", "comps": [1, 0, 0, 0.5]}}),
            4,
            3,
        )
        .unwrap();
        assert_eq!(p.pixels.len(), 12, "the page covers its rect exactly");
        for px in &p.pixels {
            assert!(
                (px[0] - 0.5).abs() < 1e-6,
                "red must be PREMULTIPLIED by alpha 0.5, got {px:?}"
            );
            assert_eq!(px[3], 0.5, "alpha is carried straight");
        }
    }

    /// `none` is served, not refused, and contributes nothing.
    #[test]
    fn none_pages_are_fully_transparent() {
        let p = page(json!({"type": "none"}), 3, 3).unwrap();
        assert_eq!(p.pixels.len(), 9);
        assert!(
            p.pixels.iter().all(|px| *px == [0.0; 4]),
            "none writes nothing"
        );
    }

    /// Pixels are sampled at their CENTRES, so the extreme centres sit a
    /// half-pixel inside the box and land at t = ½/n and 1 − ½/n rather
    /// than at 0 and 1.
    ///
    /// This is asserted numerically because it is the kind of off-by-a-
    /// half that a reviewer cannot see and a "looks like a gradient" test
    /// cannot catch. With an 8-tall box and a black→white ramp running
    /// bottom→top (angleDeg 90 default), the TOP row's centre is at
    /// t = 1 − 1/16 = 0.9375 and the BOTTOM row's at t = 1/16 = 0.0625.
    /// A corner-sampling implementation would give 1.0 and 0.0.
    #[test]
    fn gradient_pages_sample_at_pixel_centres() {
        let p = page(
            json!({"type": "gradient", "gradient": {"kind": "linear", "stops": [
                {"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]}},
                {"pos": 1, "color": {"model": "rgb", "comps": [1, 1, 1, 0]}}]}}),
            1,
            8,
        )
        .unwrap();
        // Row 0 is the TOP in raster order, and the 90° axis runs upward,
        // so the top row carries the LAST stop's end of the ramp.
        let top = p.pixels[0][0];
        let bottom = p.pixels[7][0];
        assert!(
            (top - 0.9375).abs() < 1e-5,
            "top row centre must sample t = 1 - 1/16 = 0.9375, got {top}              (1.0 would mean corner sampling)"
        );
        assert!(
            (bottom - 0.0625).abs() < 1e-5,
            "bottom row centre must sample t = 1/16 = 0.0625, got {bottom}"
        );
    }

    /// §22.5 pattern content refuses TYPED — it is never rendered as a
    /// wrong picture and never silently omitted.
    #[test]
    fn pattern_content_refuses_typed_at_the_composite_leg() {
        let e = page(
            json!({"type": "pattern", "patternRef": "assets/paper"}),
            4,
            4,
        )
        .unwrap_err();
        assert_eq!(e.code.as_str(), "error.command.not_implemented");
        assert!(
            e.message.contains("pattern"),
            "names its cause: {}",
            e.message
        );
    }

    /// A `noise` gradient NEVER produces a page, whatever the box does —
    /// it refuses typed rather than being aliased onto another kind.
    ///
    /// Both box shapes are walked because they refuse through DIFFERENT
    /// gates, and the point is that neither gate lets an unservable kind
    /// through as a silent success:
    ///   * a valid box reaches the kind probe → `not_implemented`, naming
    ///     `noise`;
    ///   * a degenerate box is refused by `from_layer_box` FIRST →
    ///     `out_of_range`, naming the box. That ordering is the honest
    ///     reason the empty case is safe; it is NOT the probe (an earlier
    ///     draft of this test asserted it was, and failed — the box gate
    ///     fires first, which is why `render_fill_page`'s note says so).
    #[test]
    fn noise_never_yields_a_page_whatever_the_box() {
        let noise = json!({"type": "gradient", "gradient": {"kind": "noise", "stops": [
            {"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]}},
            {"pos": 1, "color": {"model": "rgb", "comps": [1, 1, 1, 0]}}]}});

        let valid = page(noise.clone(), 4, 4).unwrap_err();
        assert_eq!(valid.code.as_str(), "error.command.not_implemented");
        assert!(
            valid.message.contains("noise"),
            "names its cause: {}",
            valid.message
        );

        let degenerate = page(noise, 0, 0).unwrap_err();
        assert_eq!(
            degenerate.code.as_str(),
            "error.schema.out_of_range",
            "a degenerate box is refused by the box gate, before the kind probe"
        );
    }
}
