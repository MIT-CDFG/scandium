// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.1/§8.2 (PNG codec: straight alpha, hidden RGB semantics),
//! §6.4 (premultiplied boundary math — resample/composite premultiplied),
//! §4.4 (source-over in linear light); plan §2.1 row "PNG import/export —
//! sRGB↔linear f32 at boundaries". The interim CP-0.1 color assumption is
//! documented in [`super::srgb`] (module doc) and the lane log.
//!
//! Import: any PNG (gray/gray-alpha/rgb/rgba/palette × 8/16-bit) decodes
//! sRGB → linear f32, premultiplies in linear (§6.4 straight→working row),
//! and lands in an **f32 Rgba linear premultiplied** buffer placed at
//! `(0, 0)` with the image's full extent (§19.1: layer buffers are
//! unbounded; the proto cropped to canvas — divergence noted).
//!
//! Export: CP-0.1 has no compositor lane; the reachable document space is
//! raster layers with `Normal` blend and full opacity (no property kinds
//! are implemented), so export composites bottom-to-top with §4.4
//! source-over in linear premultiplied f32 — complete for every reachable
//! state — then unpremultiplies and encodes sRGB 8-bit RGBA (straight
//! alpha, PNG's convention, §8.2). Non-Rgba document modes need the §5/§6
//! conversion pipeline and are refused typed until CP-0.5.
//!
//! CP-0.7 (QA-F8 [RULED]): documents with effectively-visible smart
//! objects export through a guarded arm that serves each smart layer via
//! its value-captured raster proxy (`cached_composite`, DD ⑨/D-48 —
//! baked at `smartobject.convert` / `edit_contents` / `replace`; replay
//! never re-composites the embedded document) and refuses TYPED on any
//! node the §4.4 reference math cannot represent — silent omission at a
//! composite door is banned (interim discipline, also ruled).
//!
//! CP-0.7 reopened §12.1 slice (PARAMETRIC RE-RENDER): the guarded arm
//! now EVALUATES — enabled smart-filter chains apply over the proxy in
//! §12.1 chain order with per-op masks honored, and adjustment layers
//! apply to the composite below (§17.1/§11.4 pass-through stacking) —
//! through [`crate::engine::op_eval`], with kernels the ASSEMBLY carries
//! (the op bridges register them construction-only; a bare-core assembly
//! still refuses typed on any ENABLED op). §12.4 disabled-identity is
//! byte-true; evaluation is a pure function of (params, inputs).

use crate::buffer::Buffer;
use crate::engine::error::{EngineError, ErrorCode};
use crate::engine::Engine;
use crate::geom::IVec2;
use crate::io::srgb::{srgb_decode, srgb_decode_u8, srgb_encode, unpremul};
use crate::io::{read_err, write_err};
use crate::tile::channel::ChannelKind;
use crate::tile::map::TileCoord;
use crate::tile::model::ColorModelKind;
use crate::tile::storage::TileStorage;
use crate::tile::{TILE, TILE_I32};

/// Decode PNG bytes into an f32 linear premultiplied Rgba buffer at (0,0).
/// Returns the buffer plus its pixel size.
pub fn decode_png_to_buffer(
    bytes: &[u8],
    working: crate::graph::id::ProfileId,
) -> Result<(Buffer, u32, u32), EngineError> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder
        .read_info()
        .map_err(|e| read_err("decoding PNG header", e))?;
    let buf_size = reader
        .output_buffer_size()
        .ok_or_else(|| read_err("decoding PNG", "image too large for a buffer"))?;
    let mut buf = vec![0u8; buf_size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| read_err("decoding PNG image data", e))?;
    let (w, h) = (info.width, info.height);
    let data = &buf[..info.buffer_size()];

    let channels = info.color_type.samples();
    let sixteen = info.bit_depth == png::BitDepth::Sixteen;
    let px_bytes = channels * if sixteen { 2 } else { 1 };

    let mut out = Buffer::new(ChannelKind::F32, ColorModelKind::Rgba, working);
    for y in 0..h {
        for x in 0..w {
            let i = (y as usize * w as usize + x as usize) * px_bytes;
            // 8-bit components decode via the MAJOR-3 LUT (bit-identical to
            // the scalar path); 16-bit stays scalar.
            let read = |c: usize| -> f32 {
                if sixteen {
                    u16::from_be_bytes([data[i + 2 * c], data[i + 2 * c + 1]]) as f32 / 65535.0
                } else {
                    data[i + c] as f32 / 255.0
                }
            };
            let read_linear = |c: usize| -> f32 {
                if sixteen {
                    srgb_decode(
                        u16::from_be_bytes([data[i + 2 * c], data[i + 2 * c + 1]]) as f32 / 65535.0,
                    )
                } else {
                    srgb_decode_u8(data[i + c])
                }
            };
            // §6.4 straight→working: decode TRC → linear (8-bit via the
            // MAJOR-3 LUT), then × a. Alpha is linear already.
            let (lr, lg, lb, a) = match info.color_type {
                png::ColorType::Grayscale => {
                    let v = read_linear(0);
                    (v, v, v, 1.0)
                }
                png::ColorType::GrayscaleAlpha => {
                    let v = read_linear(0);
                    (v, v, v, read(1))
                }
                png::ColorType::Rgb => (read_linear(0), read_linear(1), read_linear(2), 1.0),
                png::ColorType::Rgba => (read_linear(0), read_linear(1), read_linear(2), read(3)),
                other => {
                    return Err(read_err(
                        "decoding PNG",
                        format!("unsupported color type {other:?} after expansion"),
                    ))
                }
            };
            let px = [lr * a, lg * a, lb * a, a];
            if px.iter().any(|v| *v != 0.0) {
                out.write_pixel_f32(IVec2::new(x as i32, y as i32), px);
            }
        }
    }
    out.recompute_bounds_pixel_exact();
    Ok((out, w, h))
}

/// One layer pixel as linear premultiplied f32, honoring the interim
/// storage convention ([`super::srgb`] module doc): f32 buffers are
/// linear premult already; u8/u16 buffers carry sRGB-encoded values whose
/// premultiplied state is exercised at alpha ∈ {0, 1} only.
fn layer_pixel_linear(buf: &Buffer, p: IVec2) -> [f32; 4] {
    linearize(buf.read_pixel_f32(p), buf.kind(), buf.premultiplied())
}

/// Storage-normalized RGBA → linear premultiplied working value. This is the
/// EXACT per-pixel conversion `layer_pixel_linear` performs, factored out so
/// the pixel-major reference walk and the MAJOR-1 tile-major walk
/// ([`composite_frame_tiled`]) share one arithmetic — same match arms, same
/// expressions, same LUT — and can never drift a bit apart. `kind` and
/// `premultiplied` are the owning buffer's, hoisted per layer by the
/// tile-major caller (out of the pixel loop) and read inline by the
/// reference caller.
#[inline]
fn linearize(v: [f32; 4], kind: ChannelKind, premultiplied: bool) -> [f32; 4] {
    match kind {
        ChannelKind::F32 => {
            // Wave-2 arbitration 3 fix (render's verified conversion
            // semantics, `from_f32_dense`): premultiplied f32 passes
            // through; STRAIGHT f32 multiplies by alpha (§6.4
            // straight→working row) — the flag was previously ignored.
            if premultiplied {
                v
            } else {
                let a = v[3];
                [v[0] * a, v[1] * a, v[2] * a, a]
            }
        }
        ChannelKind::U8 => {
            // Interim u8 convention ([`super::srgb`], render's
            // `from_unsigned_dense`): sRGB-encoded components + alpha;
            // decode via the MAJOR-3 LUT (the stored normalized value is
            // exactly b/255, so the byte recovery is lossless), then
            // premultiply in linear (§6.4 straight→working).
            let a = v[3];
            let d = |c: f32| srgb_decode_u8((c * 255.0).round() as u8);
            [d(v[0]) * a, d(v[1]) * a, d(v[2]) * a, a]
        }
        _ => {
            // Feature-gated u16: scalar decode, same rule.
            let a = v[3];
            [
                srgb_decode(v[0]) * a,
                srgb_decode(v[1]) * a,
                srgb_decode(v[2]) * a,
                a,
            ]
        }
    }
}

/// Quantize + encode one canvas plane of LINEAR PREMULTIPLIED f32 RGBA
/// (row-major, w×h×4) to 8-bit sRGB straight-alpha PNG bytes — the shared
/// tail of core's reference export and the CLI's render-composited export
/// (both quantize through the same §6.4 math, so the single-layer paths
/// stay byte-compatible).
pub fn encode_composite_png(w: u32, h: u32, plane: &[f32]) -> Result<Vec<u8>, EngineError> {
    if plane.len() != w as usize * h as usize * 4 {
        return Err(write_err(
            "encoding composite",
            "plane length must be w*h*4",
        ));
    }
    let mut data = Vec::with_capacity(plane.len());
    for px in plane.chunks_exact(4) {
        let s = unpremul([px[0], px[1], px[2], px[3]]);
        data.push((srgb_encode(s[0].clamp(0.0, 1.0)) * 255.0).round() as u8);
        data.push((srgb_encode(s[1].clamp(0.0, 1.0)) * 255.0).round() as u8);
        data.push((srgb_encode(s[2].clamp(0.0, 1.0)) * 255.0).round() as u8);
        data.push((s[3].clamp(0.0, 1.0) * 255.0).round() as u8);
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(std::io::Cursor::new(&mut out), w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc
            .write_header()
            .map_err(|e| write_err("encoding PNG header", e))?;
        writer
            .write_image_data(&data)
            .map_err(|e| write_err("encoding PNG data", e))?;
    }
    Ok(out)
}

/// §4.4 source-over in linear premult, bottom-to-top, over the canvas —
/// the shared compositing loop of the reference export's two arms (legacy
/// raster-only and the QA-F8 proxy-serving arm quantize identically).
pub(crate) fn composite_plane(layers: &[&Buffer], w: u32, h: u32) -> Vec<f32> {
    composite_plane_frame(layers, crate::geom::IRect::new(0, 0, w as i32, h as i32))
}

/// The §4.4 source-over reference walk over an arbitrary output FRAME
/// (document space). `frame.min == 0` is the canvas-origin case — byte-for-
/// byte the legacy `composite_plane(layers, w, h)`; a non-zero `frame.min`
/// is the §19.1 non-destructive crop offset, so the emitted row-major
/// `frame.width × frame.height` plane SAMPLES each layer at `+frame.min`
/// (pixels outside the crop are hidden, never moved or deleted).
///
/// MAJOR-1: unmasked layers are the empty-mask case of the tile-major walk.
pub(crate) fn composite_plane_frame(layers: &[&Buffer], frame: crate::geom::IRect) -> Vec<f32> {
    let empty = crate::graph::mask_eval::ResolvedMasks::none();
    let walk: Vec<LayerWalk> = layers.iter().map(|b| LayerWalk::new(b, &empty)).collect();
    composite_frame_tiled(&walk, frame)
}

/// The §4.4 reference walk with §11.2 coverage (Sub-slice A): each visible
/// raster layer carries its ENABLED, unfeathered masks; the premultiplied
/// source is multiplied by the shared per-pixel `E`
/// ([`crate::apply_coverage`]) before source-over — the SAME `E` the
/// render compositor and GPU apply, so the two compositing surfaces agree
/// byte-for-byte on the shared serve set. A layer with NO enabled masks
/// takes the unmasked arithmetic verbatim (byte-stable — the goldens bar).
///
/// MAJOR-1: the walk is TILE-MAJOR (see [`composite_frame_tiled`]) — the
/// per-pixel-per-layer hash probe of the old pixel-major walk is hoisted to
/// once per (tile, layer). The emitted plane is BIT-IDENTICAL: same layer
/// order, same conversion, same §11.2 coverage, same source-over arithmetic
/// per pixel — only the pixel visit order and where the tile pointer comes
/// from change, and pixels do not interact.
pub(crate) fn composite_plane_frame_masked(
    layers: &[(&Buffer, Vec<&crate::graph::mask_node::MaskNode>)],
    frame: crate::geom::IRect,
) -> Vec<f32> {
    // Resolve every stack ONCE, before the walk — this is where each
    // feathered mask's single canonical Gaussian plane is built. Doing it
    // inside the tile loop would blur per output tile, which the σ > 32
    // box path makes rect-DEPENDENT (mask_eval module doc).
    let resolved: Vec<crate::graph::mask_eval::ResolvedMasks<'_>> = layers
        .iter()
        .map(|(_, masks)| crate::graph::mask_eval::ResolvedMasks::resolve(masks))
        .collect();
    let walk: Vec<LayerWalk> = layers
        .iter()
        .zip(resolved.iter())
        .map(|((b, _), masks)| LayerWalk::new(b, masks))
        .collect();
    composite_frame_tiled(&walk, frame)
}

/// One layer's compositing posture, resolved ONCE for the whole walk: its
/// storage decode rule (`kind`/`premultiplied`), its color `model` (the
/// channel count a dense read needs), and its §11.2 mask stack. Hoisting
/// these out of the pixel loop is half of the MAJOR-1 win (the other half is
/// the per-tile storage resolution in [`TileView`]).
struct LayerWalk<'a> {
    buf: &'a Buffer,
    kind: ChannelKind,
    premultiplied: bool,
    model: ColorModelKind,
    /// The layer's §11.2 stack RESOLVED once for the whole walk — enabled
    /// masks with their canonical feathered planes already built. Resolved
    /// per LAYER, never per tile: the feather's box path is not
    /// rect-invariant, so a per-tile blur would seam (mask_eval module doc).
    masks: &'a crate::graph::mask_eval::ResolvedMasks<'a>,
}

impl<'a> LayerWalk<'a> {
    #[inline]
    fn new(buf: &'a Buffer, masks: &'a crate::graph::mask_eval::ResolvedMasks<'a>) -> Self {
        LayerWalk {
            buf,
            kind: buf.kind(),
            premultiplied: buf.premultiplied(),
            model: buf.model(),
            masks,
        }
    }
}

/// One layer's storage at ONE tile coordinate, resolved once per (tile,
/// layer) — the single `HashMap<TileCoord, _>` probe the MAJOR-1 walk pays,
/// instead of one probe per pixel per layer.
enum TileView<'a> {
    /// No tile here: reads are all-zero storage (ES §2.4). Handled as a
    /// `[0.0; 4]` raw read (NOT skipped) so the source-over arithmetic runs
    /// exactly as the reference pixel-major walk did — see the bit-exactness
    /// note in [`composite_frame_tiled`].
    Absent,
    /// A `Uniform` (solid) tile: every pixel reads the same storage value.
    Uniform([f32; 4]),
    /// A dense tile: index directly with local (lx, ly), no map probe.
    Dense(&'a TileStorage),
}

/// §5 occlusion-cull predicate: the index of the TOPMOST layer that is
/// unmasked and reads FULLY OPAQUE (αs == 1.0) across the entire clipped tile
/// region `[xa,xb) × [ya,yb)` — every layer strictly below it is provably
/// occluded and can be skipped. Returns 0 when no such occluder exists (or when
/// the occluder IS layer 0), i.e. the full walk.
///
/// Only reached under the caller's `all_u8` gate, so `αs == 1.0` ⇔ the stored
/// alpha reads exactly 1.0 (`linearize` passes alpha through unchanged on every
/// arm). A MASKED layer is never an occluder — §11.2 coverage `E` can only
/// REDUCE alpha, so a masked layer's effective coverage is partial. The
/// top-down scan early-outs on the first non-opaque pixel of a dense candidate,
/// so a stack with no occluder pays at most a few alpha reads per layer.
#[inline]
fn tile_occluder_base(
    views: &[TileView<'_>],
    layers: &[LayerWalk<'_>],
    xa: i32,
    xb: i32,
    ya: i32,
    yb: i32,
    tile_left: i64,
    tile_top: i64,
) -> usize {
    for li in (0..views.len()).rev() {
        // A masked layer's coverage is partial (§11.2) — never an occluder.
        if !layers[li].masks.is_empty() {
            continue;
        }
        let opaque = match &views[li] {
            // Absent ⇒ αs = 0 everywhere ⇒ transparent, not an occluder.
            TileView::Absent => false,
            // Uniform ⇒ one alpha decides the whole tile (O(1)).
            TileView::Uniform(v) => v[3] == 1.0,
            // Dense ⇒ every clipped pixel must read alpha == 1 (early-out).
            TileView::Dense(t) => {
                let model = layers[li].model;
                let mut all_opaque = true;
                'scan: for y in ya..yb {
                    let ly = (y as i64 - tile_top) as usize;
                    for x in xa..xb {
                        let lx = (x as i64 - tile_left) as usize;
                        if t.read_pixel_f32(lx, ly, model)[3] != 1.0 {
                            all_opaque = false;
                            break 'scan;
                        }
                    }
                }
                all_opaque
            }
        };
        if opaque {
            return li;
        }
    }
    0
}

/// MAJOR-1 TILE-MAJOR composite over `frame` (document space), §4.4
/// source-over bottom-to-top with §11.2 coverage. Bit-identical to the old
/// pixel-major `composite_plane_frame[_masked]` walks and pinned so by
/// `tiled_walk_is_bitexact_vs_pixel_major` (masks, crop offsets, tile
/// boundaries, absent/Uniform tiles, u8+f32, straight+premul, α∈{0,1}, HDR).
///
/// The optimization changes ONLY (a) the order pixels are visited —
/// tile-major, in nested global-tile-grid cells clipped to `frame` — and (b)
/// where each layer's tile pointer comes from — resolved ONCE per (tile,
/// layer) via [`TileView`] rather than a per-pixel `HashMap` probe inside
/// `Buffer::read_pixel_f32`. The per-pixel expression is untouched:
/// `linearize` (the SAME conversion `layer_pixel_linear` uses), then the SAME
/// §11.2 `apply_coverage(src, layer_effective_coverage(masks, p))`, then the
/// SAME `ia = 1 − src[3]; acc[c] = src[c] + acc[c]·ia`. Pixels do not
/// interact and the output index `(oy·w + ox)` reproduces the reference's
/// row-major frame order exactly, so the emitted plane is byte-identical.
///
/// Bit-exactness of ABSENT tiles: the reference reads `[0,0,0,0]` for an
/// absent tile and runs the full arithmetic (`acc[c] = 0 + acc[c]·1`). We do
/// the SAME — an absent tile is a `[0,0,0,0]` raw read, NOT a `continue` —
/// because `0.0 + acc[c]·1.0` is only a no-op up to the sign of zero
/// (`-0.0` would flip to `+0.0` if skipped). Running the arithmetic keeps the
/// bits identical for every accumulator state.
///
/// **§5 OCCLUSION CULL (MASK-READBACK-PERF).** Per tile, the inner layer walk
/// starts at [`tile_occluder_base`] instead of layer 0: when some layer `k` is
/// unmasked and reads FULLY OPAQUE (αs == 1.0) across the whole clipped tile
/// region, every contribution below `k` is provably occluded and skipped. This
/// is BYTE-IDENTICAL, but ONLY under the `all_u8` gate (every layer is
/// [`ChannelKind::U8`]). Proof: at the opaque occluder `ia_k = 1 − 1.0 = +0.0`,
/// so the full walk computes `acc[c] = src_k[c] + acc_below[c]·(+0.0)`. For U8
/// content every linearized value lies in `[0,1]` — finite, non-negative, never
/// `-0.0` — so (a) `acc_below[c]·0.0 == +0.0` (never `NaN`: needs a non-finite
/// operand; never `-0.0`: needs a negative one) and (b) `src_k[c] + 0.0 ==
/// src_k[c]` (only `-0.0 + 0.0` would flip, and `src_k[c]` is never `-0.0`).
/// Thus the post-`k` accumulator equals `src_k` bit-for-bit — exactly what the
/// culled walk (acc starts `[0,0,0,0]`, composites `k` first) also yields. Any
/// non-U8 layer (F32/premul/HDR — where `-0.0`, negatives, `Inf`/`NaN` can
/// occur, or the multiply-by-zero could mask a real `NaN`) DISABLES the cull
/// for the whole walk, so those cases stay on the untouched exact path. Pinned
/// by `occlusion_cull_is_bitexact_vs_full_walk`.
fn composite_frame_tiled(layers: &[LayerWalk], frame: crate::geom::IRect) -> Vec<f32> {
    let (w, h) = (
        frame.width().max(0) as usize,
        frame.height().max(0) as usize,
    );
    let mut plane = vec![0.0f32; w * h * 4];
    if w == 0 || h == 0 {
        return plane;
    }

    // Global tile-grid cells intersecting the frame. Tiles tile document
    // space on the `TILE_I32` grid (`x.div_euclid(TILE)` — the exact mapping
    // `Buffer::locate` uses), so this covers every frame pixel exactly once,
    // for any frame origin including the §19.1 crop offset and negatives.
    let tx0 = frame.min.x.div_euclid(TILE_I32);
    let tx1 = (frame.max.x - 1).div_euclid(TILE_I32);
    let ty0 = frame.min.y.div_euclid(TILE_I32);
    let ty1 = (frame.max.y - 1).div_euclid(TILE_I32);

    // §5 occlusion cull gate: the reorder is provably byte-identical ONLY when
    // every layer is U8 (values in [0,1] — finite, non-negative, no -0.0). Any
    // other kind keeps the whole walk on the untouched exact path. See the
    // module-doc proof above.
    let all_u8 = layers.iter().all(|lw| lw.kind == ChannelKind::U8);

    let mut views: Vec<TileView<'_>> = Vec::with_capacity(layers.len());
    for ty in ty0..=ty1 {
        // i64 tile-origin math avoids i32 overflow at extreme crop rects.
        let tile_top = ty as i64 * TILE as i64;
        let ya = (frame.min.y as i64).max(tile_top) as i32;
        let yb = (frame.max.y as i64).min(tile_top + TILE as i64) as i32;
        for tx in tx0..=tx1 {
            let tile_left = tx as i64 * TILE as i64;
            let xa = (frame.min.x as i64).max(tile_left) as i32;
            let xb = (frame.max.x as i64).min(tile_left + TILE as i64) as i32;

            // ONE map probe per (tile, layer) — the whole point of MAJOR-1.
            let coord = TileCoord::new(0, tx, ty);
            views.clear();
            views.extend(layers.iter().map(|lw| match lw.buf.get_tile(coord) {
                None => TileView::Absent,
                Some(t) => match &**t {
                    TileStorage::Uniform { value } => TileView::Uniform(*value),
                    dense => TileView::Dense(dense),
                },
            }));

            // §5 occlusion cull: skip every layer below the topmost fully-opaque
            // unmasked occluder for this tile (byte-identical under `all_u8`; a
            // no-op returning 0 when the gate is off or no occluder exists).
            let base = if all_u8 {
                tile_occluder_base(&views, layers, xa, xb, ya, yb, tile_left, tile_top)
            } else {
                0
            };

            for y in ya..yb {
                let ly = (y as i64 - tile_top) as usize;
                let oy = (y - frame.min.y) as usize;
                for x in xa..xb {
                    let lx = (x as i64 - tile_left) as usize;
                    let ox = (x - frame.min.x) as usize;
                    let p = IVec2::new(x, y);
                    let mut acc = [0.0f32; 4];
                    for (view, lw) in views[base..].iter().zip(&layers[base..]) {
                        // SAME raw storage value the reference's
                        // `buf.read_pixel_f32(p)` would return at p.
                        let raw = match view {
                            TileView::Absent => [0.0f32; 4],
                            TileView::Uniform(v) => *v,
                            TileView::Dense(t) => t.read_pixel_f32(lx, ly, lw.model),
                        };
                        let mut src = linearize(raw, lw.kind, lw.premultiplied);
                        if !lw.masks.is_empty() {
                            src = crate::apply_coverage(src, lw.masks.coverage_at(p));
                        }
                        let ia = 1.0 - src[3];
                        for c in 0..4 {
                            acc[c] = src[c] + acc[c] * ia;
                        }
                    }
                    let o = (oy * w + ox) * 4;
                    plane[o..o + 4].copy_from_slice(&acc);
                }
            }
        }
    }
    plane
}

/// True when the document carries a PARAMETRIC layer (smart object,
/// adjustment, shape, text, or §22 fill) that
/// is EFFECTIVELY visible (own flag + every ancestor group visible) — the
/// trigger predicate for the parametric composite arm below, and the CLI
/// export dispatch's routing test. Both kinds route here to be SERVED
/// through the §12.1 evaluation (this reopened-CP-0.7 slice); what the
/// arm cannot represent refuses typed, never omits (QA-F8 discipline).
pub fn has_visible_parametric_content(doc: &crate::graph::document::Document) -> bool {
    let mut stack: Vec<crate::graph::id::LayerId> = doc
        .children_of(doc.root())
        .map(|c| c.to_vec())
        .unwrap_or_default();
    while let Some(id) = stack.pop() {
        let Some(node) = doc.layer(id) else { continue };
        if !node.visible() {
            continue; // §4.6 step 1: a hidden node hides its whole subtree
        }
        if node.smart_object().is_some()
            || node.adjustment_op().is_some()
            || node.shape_data().is_some()
            || node.text_data().is_some()
            || node.fill_data().is_some()
        {
            // Shape layers are parametric content (§20.2): routing them
            // into the guarded arm makes the reference exporter REFUSE
            // TYPED on them (its unknown-kind catch-all) instead of the
            // QA-F8 silent-omit class — until the render row's §20.2
            // exact-coverage fill serves them. TEXT layers joined this
            // route at the one-compositing-truth fix (O-UI filing,
            // O-INT interim ruling): the door-slice-2 Text kind was
            // missing here, so the raster-walk arm SILENTLY OMITTED
            // visible text from exports while readback refused typed —
            // the exact QA-F8 split this predicate exists to prevent.
            // Both surfaces now refuse the SAME typed wall; the real
            // text composite leg is the O-TEXTVEC/render row.
            //
            // §22 FILL layers joined for the identical reason, caught the
            // identical way — by testing the COMPOSITE door and not just
            // the command door. The moment `layer.add{kind:"fill"}` began
            // serving, a visible fill layer was parametric content that
            // this predicate did not name, so the raster-walk arm ran and
            // exported a document with the fill layer SILENTLY MISSING
            // while the render compositor refused it typed
            // (`CompositeError::UnsupportedSource`) — a serve↔refuse split
            // across the two doors, which is precisely what this predicate
            // exists to prevent.
            //
            // Fill layers now SERVE at both doors (the FillContent→pixels
            // leg: solid, the §22.4 ramp, `none`), and naming them here is
            // MORE load-bearing after that change, not less: the raster
            // walk cannot represent a layer that carries no pixels at all,
            // so a visible fill layer must still be routed to the
            // parametric arm — which is now where it gets DRAWN rather
            // than where it gets refused. `pattern` content and `noise`
            // gradients still refuse typed there, uniformly at both doors.
            return true;
        }
        if node.is_group() {
            if let Some(children) = doc.children_of(id) {
                stack.extend(children.iter().copied());
            }
        }
    }
    false
}

/// A layer's ENABLED raster masks (disabled skipped — the §11.2 no-op).
fn enabled_masks_of<'a>(
    doc: &'a crate::graph::document::Document,
    node: &crate::graph::node::LayerNode,
) -> Vec<&'a crate::graph::mask_node::MaskNode> {
    node.masks()
        .iter()
        .filter_map(|m| doc.mask(*m))
        .filter(|m| m.enabled())
        .collect()
}

/// §11.2 mask support check for the reference RASTER-walk arm: walk the
/// effectively-visible tree and refuse — typed, uniformly with the render
/// compositor (never a serve↔refuse split) — every mask this slice cannot
/// serve on the reference path:
///
/// * a mask on a GROUP (§4.7 row-4 isolation — the render row);
/// * an enabled mask on a non-identity-transform leaf (Sub-slice C — the
///   §18 resample row).
///
/// A raster LEAF mask on an identity-transform layer is SERVED, FEATHERED
/// OR NOT: the coverage multiply is applied in
/// [`composite_plane_frame_masked`], whose per-layer
/// [`crate::graph::mask_eval::ResolvedMasks`] carries the §11.2 Gaussian.
/// This REPLACES aud-replay F2's uniform-refusal stopgap: the QA-F8 rule
/// (serve or refuse typed, never silently drop) now serves the mask.
///
/// **Sub-slice B (feather) no longer appears in this list** — it is
/// served. What must stay true is that it is served on BOTH surfaces: the
/// render compositor's `resolve_layer_coverage` feathers through the same
/// `ResolvedMasks`, so neither door can drift into refusing while the
/// other serves.
///
/// The walk is EXACTLY [`Document::z_order_walk`] filtered by
/// [`Document::effective_visible`] — the SAME node set
/// [`composite_plane_frame_masked`] composites (§5 self-review: the check
/// must visit precisely the composited set, never a superset or subset).
/// Effective visibility (own flag + every ancestor group visible), NOT own
/// `visible()` alone: a leaf inside a HIDDEN group is not composited (the
/// hidden-group ruling — a hidden group hides its whole subtree, §4.6/§4.7),
/// so its mask must not be support-checked here either — else this check
/// could refuse typed on a mask that the composite never touches (a
/// serve↔refuse split against the render row, which likewise skips it).
fn reference_mask_support(doc: &crate::graph::document::Document) -> Result<(), EngineError> {
    let refuse = |what: String| EngineError::new(ErrorCode::NotImplemented, what);
    for id in doc.z_order_walk() {
        if !doc.effective_visible(id) {
            continue; // effective visibility — the exact composite-walk predicate
        }
        let Some(node) = doc.layer(id) else { continue };
        let masks = enabled_masks_of(doc, node);
        if !masks.is_empty() {
            if node.is_group() {
                return Err(refuse(format!(
                    "layer {id} carries a mask on a GROUP; §11.2 Sub-slice A serves \
                     unfeathered raster masks on LEAVES, but a masked group is §4.7 \
                     row-4 isolation with no reference composite path yet (the render \
                     row) — refused typed on both paths, never a split"
                )));
            }
            // FEATHER IS SERVED (Sub-slice B). A feathered mask no longer
            // refuses here: `composite_plane_frame_masked` resolves the
            // stack through `ResolvedMasks`, which builds the §11.2
            // Gaussian over one canonical rect. Only the GROUP and
            // TRANSFORM rows above/below still refuse.
            if !node.transform().is_identity() {
                return Err(refuse(format!(
                    "layer {id} carries a mask on a non-identity-transform layer; \
                     transformed-layer masks (§18 resample, linked sample space) are \
                     Sub-slice C — refused typed on both paths, never a split"
                )));
            }
        }
    }
    Ok(())
}

/// One composite source of the parametric-serving arm, in §2.6 z-order.
/// Each raster-like source carries its resolved §11.2 coverage (the
/// layer's ENABLED, unfeathered raster masks — Sub-slice A; empty ⇒
/// unmasked). Coverage is applied as the shared per-pixel multiply
/// ([`crate::apply_coverage`]) — the SAME `E` the render compositor and
/// GPU apply, so the two doors stay one compositing truth.
enum ParametricSource<'a> {
    /// A raster layer's pixels (§4.4 source-over, as ever).
    Raster {
        buf: &'a Buffer,
        masks: crate::graph::mask_eval::ResolvedMasks<'a>,
    },
    /// A smart object: the value-captured proxy (`cached_composite`,
    /// DD ⑨/D-48 — baked at convert / edit_contents / replace, never
    /// re-composited here) with its §12.2 smart-filter chain evaluated
    /// OVER it at render time (§12.1 order; disabled entries identity).
    Smart {
        source: &'a Buffer,
        chain: &'a [crate::graph::id::OpId],
        masks: crate::graph::mask_eval::ResolvedMasks<'a>,
    },
    /// An adjustment layer: the §17.1 n = 1 chain over the composite
    /// below (§11.4 pass-through stacking — the reference arm refuses
    /// isolating groups, so its reach is the whole backdrop). §11.4 is
    /// cited HERE for SCOPE only; the coverage-lerp is §12.1 and rides
    /// the OP mask inside the chain evaluator (hence no `masks` field on
    /// this variant). A LAYER mask on an adjustment has no rule in the
    /// frozen letter — refused typed above (v5.6 fold-in FI-13).
    Adjustment {
        op: crate::graph::id::OpId,
        /// `node.opacity()` — FI-13's node-opacity factor.
        opacity: f32,
        /// `node.blend()` — ruling-2 stage 2a. `Normal` is the collapse case.
        blend: crate::graph::node::BlendMode,
        /// The node's OWN layer masks; their §11.2 product is FI-13's
        /// coverage factor. Served since v5.6 fold-in FI-13 — before it,
        /// this variant deliberately carried no `masks` field because the
        /// frozen letter had no rule for the construct.
        masks: Vec<&'a crate::graph::mask_node::MaskNode>,
    },
    /// A composed premultiplied-linear page over its rect: Door 6's text
    /// page verbatim (§23.2 — fills applied text-side per run), or the
    /// shape leg's coverage × layer-fill product (Door 1 coverage,
    /// TEXTVEC fill math via the assembly seam — the two doors
    /// deliberately differ; the page type is core's and carries both).
    ComposedPage {
        page: crate::engine::text_ops::TextPageData,
        masks: crate::graph::mask_eval::ResolvedMasks<'a>,
    },
}

/// §4.4 source-over of one premultiplied-linear page onto another, over
/// the UNION of their rects (both legs zero-extend outside their own).
///
/// Used INSIDE the shape leg to lay the §20.2 stroke over the fill before
/// either reaches a compositor: the layer's page stays a single
/// raster-like leaf, so blend/opacity/mask/coverage apply to the stroked
/// shape as ONE source (a stroke is not a second layer). `Co = Cs +
/// Cb·(1 − αs)` on premultiplied values — the identical algebra the
/// compositors run, kept here because this resolver is the one place both
/// doors share.
fn page_source_over(
    src: &crate::engine::text_ops::TextPageData,
    dst: &crate::engine::text_ops::TextPageData,
) -> crate::engine::text_ops::TextPageData {
    let rect = src.rect.union(&dst.rect);
    if rect.is_empty() {
        return crate::engine::text_ops::TextPageData {
            rect: crate::geom::IRect::EMPTY,
            pixels: Vec::new(),
        };
    }
    let sample = |p: &crate::engine::text_ops::TextPageData, x: i32, y: i32| -> [f32; 4] {
        if p.rect.is_empty()
            || x < p.rect.min.x
            || y < p.rect.min.y
            || x >= p.rect.max.x
            || y >= p.rect.max.y
        {
            return [0.0; 4];
        }
        let idx = ((y - p.rect.min.y) * p.rect.width() + (x - p.rect.min.x)) as usize;
        p.pixels.get(idx).copied().unwrap_or([0.0; 4])
    };
    let mut pixels = Vec::with_capacity((rect.width() * rect.height()) as usize);
    for y in rect.min.y..rect.max.y {
        for x in rect.min.x..rect.max.x {
            let s = sample(src, x, y);
            let b = sample(dst, x, y);
            let inv = 1.0 - s[3];
            pixels.push([
                s[0] + b[0] * inv,
                s[1] + b[1] * inv,
                s[2] + b[2] * inv,
                s[3] + b[3] * inv,
            ]);
        }
    }
    crate::engine::text_ops::TextPageData { rect, pixels }
}

/// KEEP-BOTH RESOLUTION (2026-08-28): the lane added `page_source_over` exactly where
/// main carries this doc line. Taking "theirs" wholesale would have restored the lane's
/// OLDER wording -- "(shape or text)" -- SILENTLY DROPPING "§22 fill", which the
/// fill-gradient lane added earlier this wave. A MERGE THAT REGRESSES A COMMENT LEAVES
/// NO TEST TO FAIL.
/// Resolve ONE parametric layer kind (shape, text, or §22 fill) to its
/// premultiplied-linear ComposedPage ([`crate::engine::text_ops::
/// TextPageData`] — §20.2 shape fill / §23.2 text page / §22 fill
/// content). This is the SINGLE source of that resolution: the reference
/// export ([`parametric_sources`]) and the render compositor
/// (`appname-render`) both consume it, so the Door-6 (text) / Door-1
/// coverage × FillConvert (shape) / §22.4 ramp (fill) math lives in
/// exactly one place — one-math-library. The dependency direction is
/// render→core (production), so render reuses THIS public entry and never
/// re-derives it (no cycle).
///
/// Returns `Ok(None)` for a kind that is NOT shape/text/fill (the caller
/// routes raster/smart/adjustment/group elsewhere); `Ok(Some(page))` for a
/// served layer; `Err` (typed) for one this slice cannot serve — refused,
/// never omitted (QA-F8):
///
/// * a shape stroke carrying `dashes` / `profile`, or a non-solid stroke
///   paint (§20.2 rules 4/6 and the §22 binding — [`ShapeStrokeStyle::
///   from_wire`] owns that enumeration);
/// * a non-solid shape fill type — awaits the §22 FillContent binding;
/// * the vector service (Door 1) or fill-math routing unregistered on the
///   assembly (a shape layer present);
/// * the text service (Door 6) unregistered (a text layer present);
/// * §22.5 `pattern` fill content, and a `noise` gradient — see
///   [`crate::engine::fill_reg::render_fill_page`] for why each has
///   nothing to be synthesized from.
///
/// Compositing props (blend/opacity/mask/transform) are NOT this resolver's
/// concern — it produces the layer's OWN premultiplied page; the caller
/// applies §4.4–§4.7 to it (the reference export refuses non-default props
/// upstream, the render compositor serves them through its kernels).
pub fn resolve_parametric_page(
    doc: &crate::graph::document::Document,
    kind: &crate::graph::node::LayerKind,
    text: Option<&std::sync::Arc<dyn crate::engine::text_ops::TextLayout>>,
    vector: Option<&std::sync::Arc<dyn crate::engine::vector_ops::VectorGeometry>>,
    fill_convert: Option<crate::engine::kind_impl::FillConvertRouting>,
) -> Result<Option<crate::engine::text_ops::TextPageData>, EngineError> {
    use crate::graph::node::LayerKind;
    let refuse = |what: String| EngineError::new(ErrorCode::NotImplemented, what);
    match kind {
        LayerKind::Shape(sd) => {
            // Door 1's rasterize_fill / shape_stroke_coverage are COVERAGE
            // ONLY (§20.2/§13.1); each leg's FillContent converts +
            // premultiplies through the assembly-injected TEXTVEC fill math
            // — core re-derives nothing.
            //
            // STROKE (§20.2 `layer.stroke`): resolved off the engine-opaque
            // payload FIRST, so an unservable knob (dashes / width profile /
            // gradient-or-pattern stroke paint) refuses typed BEFORE any
            // geometry runs. `Ok(None)` = no visible stroke (absent payload,
            // `widthPx` 0, `fill: {type: none}`) — the contract's own
            // shape_add default carries `stroke: {widthPx: 0}`, and that
            // fill-only page is byte-for-byte what this door produced before
            // the stroke leg landed.
            let stroke_style = crate::engine::vector_ops::ShapeStrokeStyle::from_wire(sd.stroke())?;
            match sd.fill().get("type").and_then(serde_json::Value::as_str) {
                None | Some("solid") | Some("none") => {}
                Some(other) => {
                    return Err(refuse(format!(
                        "shape fill type \"{other}\" awaits the §22 FillContent \
                         binding (the text-v1 gap, shared posture) — refused typed"
                    )))
                }
            }
            let Some(svc) = vector else {
                return Err(crate::engine::vector_ops::geometry_unregistered(
                    "reference composite (shape layer present)",
                ));
            };
            let Some(fc) = fill_convert else {
                return Err(refuse(
                    "the fill-math seam is unregistered on this assembly — the \
                     assembly point injects appname-text's fill_to_linear_rgba/\
                     premultiply_coverage (composite-leg slice)"
                        .into(),
                ));
            };
            let sidecar = doc
                .path(sd.path())
                .ok_or_else(|| {
                    EngineError::bad_target(format!(
                        "shape path {} does not resolve (§20.2)",
                        sd.path()
                    ))
                })?
                .sidecar()
                .to_vec();
            let cov = svc.rasterize_fill(&sidecar)?;
            let fill_opt = if sd.fill().is_null() {
                None
            } else {
                Some(sd.fill())
            };
            let straight = (fc.fill_to_linear_rgba)(fill_opt);
            let pixels: Vec<[f32; 4]> = cov
                .values
                .iter()
                .map(|c| (fc.premultiply_coverage)(straight, c.clamp(0.0, 1.0)))
                .collect();
            let fill_page = crate::engine::text_ops::TextPageData {
                rect: cov.rect,
                pixels,
            };
            let Some(style) = stroke_style else {
                // Fill-only: the exact page this door has always produced.
                return Ok(Some(fill_page));
            };
            let stroke_cov = svc.shape_stroke_coverage(&sidecar, style)?;
            let Some(paint) = crate::engine::vector_ops::stroke_paint(sd.stroke())? else {
                return Ok(Some(fill_page)); // `from_wire` already ruled this out
            };
            let stroke_straight = (fc.fill_to_linear_rgba)(Some(&paint));
            let stroke_pixels: Vec<[f32; 4]> = stroke_cov
                .values
                .iter()
                .map(|c| (fc.premultiply_coverage)(stroke_straight, c.clamp(0.0, 1.0)))
                .collect();
            let stroke_page = crate::engine::text_ops::TextPageData {
                rect: stroke_cov.rect,
                pixels: stroke_pixels,
            };
            // §20.2 PAINT ORDER [PINNED]: the stroke paints OVER the fill
            // INSIDE the layer's own page — the SVG/§20.2 order ("a path +
            // fill + stroke"; an `inside`-aligned stroke must cover the fill
            // edge it shares, which is only true this way round). Both legs
            // are premultiplied linear, so the combine is the ordinary §4.4
            // source-over the compositor applies to everything else; the
            // page the caller receives is a single opaque-to-it raster-like
            // leaf, exactly as the fill-only page was.
            Ok(Some(page_source_over(&stroke_page, &fill_page)))
        }
        LayerKind::Fill(fd) => {
            // §22 FILL layer → pixels. The layer BOX is the full CANVAS,
            // and specifically `canvas_rect()` and NOT `image_rect()`.
            //
            // Two independent reasons, both checkable:
            //  1. `layer.add.yaml`'s dirtyRect row for fill kinds is
            //     "downstream ROI (§11.6; unmasked = full canvas)" — the
            //     recorder already treats a fill layer as covering the
            //     canvas, and the composite must agree with what the
            //     recorder staled or a dirty recomposite would leave
            //     stale pixels.
            //  2. §19.1's non-destructive crop HIDES pixels, never
            //     changes them. `image_rect()` is the crop window; deriving
            //     the gradient's box from it would make the ramp REFLOW
            //     when a crop is applied, so cropping would alter the
            //     colours inside the region the user kept — destructive in
            //     effect, which is exactly what §19.1 forbids. Anchoring on
            //     the canvas keeps a crop a pure window.
            //     `fill_gradient_is_invariant_under_a_non_destructive_crop`
            //     pins this; it fails if this line says `image_rect()`.
            //
            // The caller frames on `image_rect()` as usual and the page is
            // intersected with it, so a canvas-sized page is simply
            // clipped — the two rects do not need to agree.
            Ok(Some(crate::engine::fill_reg::render_fill_page(
                fd.content(),
                doc.geometry().canvas_rect(),
            )?))
        }
        LayerKind::Text(data) => match text {
            // The Door-6 serve: compose the page here (read door — no
            // envelope, no capture table; determinism = §23.2 purity in
            // (sidecar, fonts), posture recorded in the lane log).
            Some(svc) => {
                let (page, _metrics) = svc.layout_and_compose(data.sidecar())?;
                Ok(Some(page))
            }
            None => Err(crate::engine::text_ops::text_unregistered(
                "reference composite (text layer present)",
            )),
        },
        _ => Ok(None),
    }
}

/// QA-F8 discipline [RULED]: a composite door either SERVES a node's
/// content or REFUSES typed — silent omission is banned. Collect the
/// composite sources of a parametric document in §2.6 z-order
/// (bottom-to-top), refusing typed on anything this reference path
/// cannot represent (non-default compositing props and isolating groups
/// need the §4.5/§4.7 render kernels). Smart-filter chains and
/// adjustment layers are collected for §12.1 evaluation — the CP-0.7
/// reopened slice flipped those arms from typed refusal to serving.
fn parametric_sources<'a>(
    doc: &'a crate::graph::document::Document,
    text: Option<&std::sync::Arc<dyn crate::engine::text_ops::TextLayout>>,
    vector: Option<&std::sync::Arc<dyn crate::engine::vector_ops::VectorGeometry>>,
    fill_convert: Option<crate::engine::kind_impl::FillConvertRouting>,
) -> Result<Vec<ParametricSource<'a>>, EngineError> {
    use crate::graph::node::{BlendMode, LayerKind};
    let refuse = |what: String| EngineError::new(ErrorCode::NotImplemented, what);
    let mut out = Vec::new();
    // Bottom-to-top: children_of lists index 0 first (composites first);
    // a reversed stack pops in that order, groups splicing their children
    // in place (pass-through).
    let mut stack: Vec<crate::graph::id::LayerId> = doc
        .children_of(doc.root())
        .map(|c| c.iter().rev().copied().collect())
        .unwrap_or_default();
    while let Some(id) = stack.pop() {
        let node = doc
            .layer(id)
            .ok_or_else(|| EngineError::bad_target(format!("layer id {id} does not resolve")))?;
        if !node.visible() {
            continue; // exact skip (§4.6 step 1)
        }
        if node.opacity() == 0.0 {
            continue; // αs = 0 ⇒ source-over identity (§4.4) — exact skip
        }
        // §11.2 Sub-slice A: the layer's ENABLED raster masks (disabled
        // skipped — the §11.2 no-op).
        let masks: Vec<&crate::graph::mask_node::MaskNode> = node
            .masks()
            .iter()
            .filter_map(|m| doc.mask(*m))
            .filter(|m| m.enabled())
            .collect();
        if node.is_group() {
            let isolated = matches!(node.kind(), LayerKind::Group { isolated: true });
            if isolated
                || node.blend() != BlendMode::Normal
                || node.opacity() < 1.0
                || !masks.is_empty()
            {
                // A masked group is §4.7 row 4 (isolate) — isolated-group
                // compositing at the reference door is the render row (the
                // §11.2 masked-group intermediate rides it), refused typed
                // uniformly with the render compositor. Never a split.
                return Err(refuse(format!(
                    "group \"{}\" isolates (§4.7 rows 1-4); isolated-group compositing at \
                     the reference export door is the render row — QA-F8 discipline: \
                     refuse typed, never omit",
                    node.name()
                )));
            }
            if let Some(children) = doc.children_of(id) {
                stack.extend(children.iter().rev());
            }
            continue;
        }
        // Non-default compositing props still refuse (the parametric
        // reference is §4.4 source-over only) — but MASKS no longer refuse
        // here: a raster leaf mask SERVES, feathered (Sub-slice B) or not
        // (Sub-slice A).
        // FI-13 [NORMATIVE] (:5956-5962): an ADJUSTMENT node's own opacity
        // folds into the §12.1 lerp weight rather than into a source alpha,
        // so it no longer refuses here. Its blend and fill_opacity still do,
        // and for DIFFERENT reasons — see the Adjustment arm below. Every
        // other kind keeps the full source-over-only gate unchanged.
        let is_adjustment = matches!(node.kind(), LayerKind::Adjustment(_));
        // RULING 2 (2026-08-28): an Adjustment's own blend is SERVED as stage 2a
        // of the engine-binding interpretation, so it no longer refuses here.
        // Dissolve alone still refuses, in the Adjustment arm below.
        if (!is_adjustment && node.blend() != BlendMode::Normal)
            || (!is_adjustment && node.opacity() != 1.0)
            || node.fill_opacity() != 1.0
            || node.clip_to_below()
            || !node.transform().is_identity()
        {
            return Err(refuse(format!(
                "layer \"{}\" carries non-default compositing props; the parametric \
                 reference export implements §4.4 source-over only (the §4.5/§4.7 kernels \
                 are the render row) — QA-F8 discipline: refuse typed, never omit",
                node.name()
            )));
        }
        // FEATHER IS SERVED (Sub-slice B) — the mask stack below is
        // resolved through `ResolvedMasks`, which feathers.
        match node.kind() {
            // Resolve ONCE per layer (the feather's canonical plane is
            // built here, not per pixel and not per tile).
            LayerKind::Raster(buf) => out.push(ParametricSource::Raster {
                buf,
                masks: crate::graph::mask_eval::ResolvedMasks::resolve(&masks),
            }),
            LayerKind::SmartObject(so) => {
                if !so.place_transform().is_identity() {
                    return Err(refuse(
                        "non-identity smart-object placement composites through the \
                         tools resample engines (§18.2) — refused typed, never omitted"
                            .into(),
                    ));
                }
                out.push(ParametricSource::Smart {
                    source: so.cached_composite(),
                    chain: so.smart_filters(),
                    masks: crate::graph::mask_eval::ResolvedMasks::resolve(&masks),
                });
            }
            LayerKind::Adjustment(op) => {
                // FI-13 [NORMATIVE] SERVED (ENGINEERING-SPEC :5956-5962,
                // frozen at v5.6-freeze). The layer-mask refusal that stood
                // here is LIFTED, and the letter now answers the argument it
                // rested on: §12.1's tail defers the node's own masks to
                // "§11.2, §4.7", but §11.2 multiplies coverage into a SOURCE's
                // composited alpha before a §4.4 source-over, which does not
                // reduce to identity at full coverage on a node §4.6 step 6
                // writes back in place. FI-13 agrees and supplies the rule:
                // "Adjustment nodes take the §12.1 weight, not the §11.2
                // route", the §11.2 mechanism "MUST NOT be used", and
                //
                //     w = node.opacity · Π coverage(node.masks) · opᵢ.opacity · mᵢ
                //
                // Served through the SINGLE implementation both doors call
                // (`fold_adjustment_node_weight`) — never re-derived here, so
                // this door and the render compositor cannot drift.
                //
                // node.blend and node.fill_opacity still refuse, above and
                // for different reasons: blend is UNDER-DETERMINED (FI-13's
                // prose folds it in but its formula carries no blend term and
                // §4.6 step 6 does not say how it composes with opᵢ.blend —
                // routed to the operator), while fill_opacity has NO PATH at
                // all in the frozen letter. A feathered layer mask refuses
                // above with the SHARED message (§11.2 Sub-slice B), the same
                // refusal every other leaf kind takes.
                if matches!(node.blend(), BlendMode::Dissolve) {
                    return Err(refuse(format!(
                        "adjustment layer \"{}\" carries blend Dissolve; ruling 2's stage 2a is \
                         a COLOUR-ONLY blend and Dissolve is a stochastic COVERAGE threshold \
                         (§4.5) routing to neither kernel arm — refused typed, never omitted",
                        node.name()
                    )));
                }
                out.push(ParametricSource::Adjustment {
                    op: *op,
                    opacity: node.opacity(),
                    blend: node.blend(),
                    masks,
                })
            }
            LayerKind::Shape(_) | LayerKind::Text(_) | LayerKind::Fill(_) => {
                // The SHAPE/TEXT/FILL legs (Door 1 coverage×FillConvert /
                // Door 6 text page / the §22.4 ramp) resolve through the
                // SINGLE public resolver ([`resolve_parametric_page`]) —
                // the same math the render compositor consumes
                // (one-math-library, dep direction render→core; the two
                // doors never re-derive each other).
                let page = resolve_parametric_page(doc, node.kind(), text, vector, fill_convert)?
                    .expect("shape/text/fill kinds resolve to a ComposedPage");
                out.push(ParametricSource::ComposedPage {
                    page,
                    masks: crate::graph::mask_eval::ResolvedMasks::resolve(&masks),
                });
            }
            other => {
                return Err(refuse(format!(
                    "layer kind \"{}\" has no reference composite path — refused typed, \
                     never omitted (QA-F8 discipline)",
                    other.kind_str()
                )));
            }
        }
    }
    Ok(out)
}

/// RULING 2 (operator, 2026-08-28) STAGE 2a at the reference door — apply the
/// Adjustment node's own `blend` between the backdrop and the chain result,
/// COLOUR ONLY, alpha held at `a0`.
///
/// The kernels arrive by the ASSEMBLY-INJECTED §14.4 seam
/// (`with_blend_kernels`), which is render's own routing table — core never
/// re-derives a blend. That is why this door and the render compositor cannot
/// disagree: they are calling the same two function pointers.
///
/// ★ At `Normal` this returns immediately and touches nothing, so FI-13's
/// published path is byte-identical and the collapse is STRUCTURAL.
///
/// `None` routing (a bare assembly) also returns untouched: a bare assembly
/// cannot reach here, because the arms that need kernels answer the typed NI
/// mirror upstream.
fn blend_node_unit(
    backdrop: &crate::engine::op_eval::OpPlane,
    served: &mut crate::engine::op_eval::OpPlane,
    mode: crate::graph::node::BlendMode,
    routing: Option<crate::engine::kind_impl::BlendKernelRouting>,
) {
    if mode == crate::graph::node::BlendMode::Normal {
        return;
    }
    let Some(r) = routing else { return };
    let sep = (r.separable)(mode);
    let tri = (r.triple)(mode);
    for (b, t) in backdrop.px.iter().zip(served.px.iter_mut()) {
        let a = b[3];
        if a <= 0.0 {
            *t = *b;
            continue;
        }
        let inv = 1.0 / a;
        let bs = [b[0] * inv, b[1] * inv, b[2] * inv];
        let ts = [t[0] * inv, t[1] * inv, t[2] * inv];
        let out = if let Some(f) = sep {
            [f(bs[0], ts[0]), f(bs[1], ts[1]), f(bs[2], ts[2])]
        } else if let Some(f) = tri {
            f(bs, ts)
        } else {
            ts
        };
        *t = [out[0] * a, out[1] * a, out[2] * a, a];
    }
}

/// Materialize a source buffer as an [`OpPlane`] over `rect` (linear
/// premultiplied f32; absent tiles read 0 — buffers are unbounded §19.1,
/// so reads outside the canvas are real content, not fabricated edges).
fn materialize_plane(buf: &Buffer, rect: crate::geom::IRect) -> crate::engine::op_eval::OpPlane {
    let mut plane = crate::engine::op_eval::OpPlane::transparent(rect);
    let mut i = 0;
    for y in rect.min.y..rect.max.y {
        for x in rect.min.x..rect.max.x {
            plane.px[i] = layer_pixel_linear(buf, IVec2::new(x, y));
            i += 1;
        }
    }
    plane
}

/// The §12.1 PARAMETRIC RE-RENDER arm of the reference export: composite
/// the collected sources bottom-to-top in linear premultiplied f32,
/// evaluating smart-filter chains over their value-captured proxies and
/// adjustment layers over the accumulated backdrop, through the
/// assembly-provided op registry (the same construction-only seam the
/// `KindImpl`s ride — core computes with kernels the assembly carries).
///
/// ROI discipline: the accumulator starts over the widest rect any
/// adjustment's chain needs (rects composed backward through the stack,
/// §12.1 `roi_out_to_in`) and shrinks through each adjustment's
/// evaluation, so every kernel tap inside the kept region reads real
/// pixels. Raster/smart sources composite pointwise (§4.4 source-over),
/// which commutes with the crop. Pure in (params, inputs) — two runs
/// over the same document are byte-identical (D-48; pinned end-to-end).
pub(crate) fn parametric_composite_plane(
    doc: &crate::graph::document::Document,
    ops: &crate::engine::op_impl::OpAssembly,
    text: Option<&std::sync::Arc<dyn crate::engine::text_ops::TextLayout>>,
    vector: Option<&std::sync::Arc<dyn crate::engine::vector_ops::VectorGeometry>>,
    fill_convert: Option<crate::engine::kind_impl::FillConvertRouting>,
    blend_kernels: Option<crate::engine::kind_impl::BlendKernelRouting>,
    canvas: crate::geom::IRect,
) -> Result<Vec<f32>, EngineError> {
    use crate::engine::op_eval::{ChainEvaluator, OpPlane};
    let sources = parametric_sources(doc, text, vector, fill_convert)?;
    let ev = ChainEvaluator::new(ops, doc);

    // Backward pass: the rect the accumulator must span BEFORE element i
    // composites, so the topmost element lands exactly on the canvas.
    let n = sources.len();
    let mut in_rects = vec![canvas; n + 1];
    for i in (0..n).rev() {
        in_rects[i] = match &sources[i] {
            ParametricSource::Adjustment { op, .. } => {
                ev.chain_input_rect(std::slice::from_ref(op), in_rects[i + 1])?
            }
            _ => in_rects[i + 1],
        };
    }

    // Forward pass: §4.4 source-over accumulation, §12.1 evaluation, with
    // §11.2 coverage applied to each raster-like source (the SAME per-pixel
    // multiply the render compositor and GPU apply — one compositing truth).
    let mut acc = OpPlane::transparent(in_rects[0]);
    for (i, src) in sources.iter().enumerate() {
        match src {
            ParametricSource::Raster { buf, masks } => {
                composite_buffer_over(&mut acc, buf, masks);
            }
            ParametricSource::Smart {
                source,
                chain,
                masks,
            } => {
                let src_rect = ev.chain_input_rect(chain, acc.rect)?;
                let plane = materialize_plane(source, src_rect);
                let mut served = ev.eval_chain(chain, plane, acc.rect)?;
                apply_coverage_to_plane(&mut served, masks);
                composite_plane_over(&mut acc, &served);
            }
            ParametricSource::Adjustment {
                op,
                opacity,
                blend,
                masks,
            } => {
                let out_rect = in_rects[i + 1];
                // FI-13: keep the pre-adjustment backdrop over the OUTPUT rect
                // so the node factors fold into the same lerp. Cloned only
                // when the weight is not identically 1, so the pre-FI-13 path
                // allocates exactly as it did before.
                // RULING 2: the backdrop is needed for stage 2a (blend) as well
                // as stage 2b (the fold), so it is kept whenever EITHER is live.
                let needs_backdrop = !masks.is_empty()
                    || *opacity != 1.0
                    || *blend != crate::graph::node::BlendMode::Normal;
                let backdrop = if needs_backdrop {
                    Some(acc.clone().crop(out_rect))
                } else {
                    None
                };
                acc = ev.eval_chain(std::slice::from_ref(op), acc, out_rect)?;
                if let Some(b) = backdrop {
                    // stage 2a — COLOUR ONLY, alpha stays a0. Normal returns the
                    // source untouched, so the FI-13 path is byte-identical.
                    blend_node_unit(&b, &mut acc, *blend, blend_kernels);
                    // stage 2b — the FI-13 fold, unchanged.
                    crate::engine::op_eval::fold_adjustment_node_weight(
                        &b, &mut acc, *opacity, masks,
                    );
                }
            }
            ParametricSource::ComposedPage { page, masks } => {
                composite_text_page_over(&mut acc, page, masks);
            }
        }
    }
    let acc = if acc.rect == canvas {
        acc
    } else {
        acc.crop(canvas)
    };
    Ok(acc.px.into_iter().flatten().collect())
}

/// Bake a Door-6 page into a raster buffer at the document's storage
/// (the rasterize serve): premultiplied linear page → f32 verbatim, or
/// integer premul-normalized `enc(c/a)·a` (the corrected §6.4 posture —
/// c ≤ a by construction).
pub(crate) fn buffer_from_text_page(
    page: &crate::engine::text_ops::TextPageData,
    depth: ChannelKind,
    profile: crate::graph::id::ProfileId,
) -> Buffer {
    let mut buf = Buffer::new(depth, ColorModelKind::Rgba, profile);
    let integer = depth != ChannelKind::F32;
    let r = page.rect;
    let w = r.width() as usize;
    for (i, px) in page.pixels.iter().enumerate() {
        let a = px[3].clamp(0.0, 1.0);
        if a <= 0.0 {
            continue; // §2.3 absence = zero
        }
        let out = if integer {
            let inv = 1.0 / a;
            [
                (srgb_encode((px[0] * inv).clamp(0.0, 1.0)) * a).clamp(0.0, a),
                (srgb_encode((px[1] * inv).clamp(0.0, 1.0)) * a).clamp(0.0, a),
                (srgb_encode((px[2] * inv).clamp(0.0, 1.0)) * a).clamp(0.0, a),
                a,
            ]
        } else {
            [
                px[0].clamp(0.0, a),
                px[1].clamp(0.0, a),
                px[2].clamp(0.0, a),
                a,
            ]
        };
        let x = r.min.x + (i % w) as i32;
        let y = r.min.y + (i / w) as i32;
        buf.write_pixel_f32(IVec2::new(x, y), out);
    }
    buf.set_bounds_recorded(r);
    buf.recompute_bounds_pixel_exact();
    buf
}

/// §4.4 source-over of a Door-6 page into the accumulator over the
/// intersection of the page rect and the accumulator's rect (both planes
/// premultiplied linear — the compositor domain).
fn composite_text_page_over(
    acc: &mut crate::engine::op_eval::OpPlane,
    page: &crate::engine::text_ops::TextPageData,
    masks: &crate::graph::mask_eval::ResolvedMasks<'_>,
) {
    let span = acc.rect.intersect(&page.rect);
    if span.is_empty() {
        return;
    }
    let (aw, pw) = (acc.rect.width() as usize, page.rect.width() as usize);
    for y in span.min.y..span.max.y {
        for x in span.min.x..span.max.x {
            let ai = (y - acc.rect.min.y) as usize * aw + (x - acc.rect.min.x) as usize;
            let pi = (y - page.rect.min.y) as usize * pw + (x - page.rect.min.x) as usize;
            let mut src = page.pixels[pi];
            // §11.2: mask the premultiplied page pixel (empty ⇒ no-op).
            if !masks.is_empty() {
                src = crate::apply_coverage(src, masks.coverage_at(IVec2::new(x, y)));
            }
            let ia = 1.0 - src[3];
            for (a, s) in acc.px[ai].iter_mut().zip(&src) {
                *a = s + *a * ia;
            }
        }
    }
}

/// §4.4 source-over of a buffer's pixels into the accumulator, per pixel
/// over the accumulator's rect (identical arithmetic to the legacy
/// [`composite_plane`] walk — the pure-proxy path stays byte-stable when
/// `masks` is empty). §11.2: when the layer carries ENABLED masks, the
/// premultiplied source is multiplied by the shared per-pixel `E`
/// ([`crate::apply_coverage`]) — the SAME `E` the render/GPU apply.
fn composite_buffer_over(
    acc: &mut crate::engine::op_eval::OpPlane,
    buf: &Buffer,
    masks: &crate::graph::mask_eval::ResolvedMasks<'_>,
) {
    let rect = acc.rect;
    let mut i = 0;
    for y in rect.min.y..rect.max.y {
        for x in rect.min.x..rect.max.x {
            let p = IVec2::new(x, y);
            let mut src = layer_pixel_linear(buf, p);
            if !masks.is_empty() {
                src = crate::apply_coverage(src, masks.coverage_at(p));
            }
            let ia = 1.0 - src[3];
            for (a, s) in acc.px[i].iter_mut().zip(&src) {
                *a = s + *a * ia;
            }
            i += 1;
        }
    }
}

/// Multiply an evaluated plane by §11.2 coverage in place — the served
/// smart-chain output masked (the SAME per-pixel `E` the render/GPU
/// apply). No-op when `masks` is empty.
fn apply_coverage_to_plane(
    plane: &mut crate::engine::op_eval::OpPlane,
    masks: &crate::graph::mask_eval::ResolvedMasks<'_>,
) {
    if masks.is_empty() {
        return;
    }
    let rect = plane.rect;
    let mut i = 0;
    for y in rect.min.y..rect.max.y {
        for x in rect.min.x..rect.max.x {
            let e = masks.coverage_at(IVec2::new(x, y));
            plane.px[i] = crate::apply_coverage(plane.px[i], e);
            i += 1;
        }
    }
}

/// §4.4 source-over of an evaluated plane (same rect) into the accumulator.
fn composite_plane_over(
    acc: &mut crate::engine::op_eval::OpPlane,
    src: &crate::engine::op_eval::OpPlane,
) {
    debug_assert_eq!(acc.rect, src.rect, "served plane spans the accumulator");
    for (a, s) in acc.px.iter_mut().zip(&src.px) {
        let ia = 1.0 - s[3];
        for c in 0..4 {
            a[c] = s[c] + a[c] * ia;
        }
    }
}

/// Export the canvas composite as 8-bit sRGB straight-alpha RGBA PNG bytes.
///
/// Two arms, one quantizer: documents WITHOUT effectively-visible
/// parametric content (smart objects / adjustment layers) keep the CP-0.1
/// raster walk byte-for-byte (the goldens bar); documents WITH it take
/// the §12.1 PARAMETRIC RE-RENDER arm — smart layers serve their
/// value-captured proxy with the enabled smart-filter chain evaluated
/// over it, adjustment layers apply to the composite below (§17.1), both
/// through the assembly-provided op registry — and any node the arm
/// cannot represent refuses typed (silent omission is banned, QA-F8).
pub fn export_png_bytes(engine: &Engine) -> Result<Vec<u8>, EngineError> {
    let (w, h, plane) = composite_plane_f32(engine)?;
    encode_composite_png(w, h, &plane)
}

/// The f32 composite readback door (O-IO door 4, door slice 3): the
/// EXACT composite math of [`export_png_bytes`] — same two arms, same
/// gates — returning the LINEAR PREMULTIPLIED f32 canvas plane
/// (row-major, w×h×4) instead of encoded bytes, so the CLI/cabi can
/// build an `ExportSource` (and W21-style non-PNG exports) without the
/// precision-lossy 8-bit PNG round trip. `export_png_bytes` is now this
/// door + [`encode_composite_png`] — one math path, byte-stable by
/// construction.
pub fn composite_plane_f32(engine: &Engine) -> Result<(u32, u32, Vec<f32>), EngineError> {
    let doc = engine
        .document()
        .ok_or_else(|| EngineError::bad_target("an uninitialized document has no composite"))?;
    if doc.color().mode != ColorModelKind::Rgba {
        return Err(EngineError::new(
            ErrorCode::NotImplemented,
            "composite readback of non-rgba document modes needs the §5/§6 conversion \
             pipeline arm (mode-convert door C5 covers documents converted back to rgba)",
        ));
    }
    // §19.1 one compositing truth: the output frame is `image_rect()` —
    // the crop sub-region while a crop is active (origin + size honored),
    // else the full canvas (crop-free exports stay byte-identical).
    let frame = doc.geometry().image_rect();
    let (w, h) = (frame.width() as u32, frame.height() as u32);

    let plane = if has_visible_parametric_content(doc) {
        parametric_composite_plane(
            doc,
            engine.assembly().ops(),
            engine.assembly().text.as_ref(),
            engine.assembly().vector.as_ref(),
            engine.assembly().fill_convert,
            engine.assembly().blend_kernels,
            frame,
        )?
    } else {
        // §11.2 (REPLACES aud-replay F2's uniform-refusal stopgap):
        // serve raster LEAF masks on the reference raster walk, FEATHERED
        // (Sub-slice B) or not (Sub-slice A); refuse transformed leaf
        // masks and masked GROUPS typed — uniformly with the render
        // compositor, so a mask never splits serve↔refuse. The mask is
        // applied (the coverage multiply), never silently dropped (the
        // QA-F8 rule, now SERVED).
        reference_mask_support(doc)?;
        // EFFECTIVELY-visible raster layers with their ENABLED masks,
        // bottom-to-top (§2.6 z-order) — the CP-0.1 reference walk, now
        // coverage-aware; the crop offset (if any) rides `frame.min`. The
        // filter is `effective_visible` (own flag + every ancestor group
        // visible), NOT own `visible()` alone: an own-visible leaf inside a
        // HIDDEN group is skipped by the render compositor (a hidden group
        // hides its whole subtree, §4.6/§4.7), so the reference must skip it
        // too — else export-vs-preview diverges (the hidden-group ruling).
        let layers: Vec<(&Buffer, Vec<&crate::graph::mask_node::MaskNode>)> = doc
            .z_order_walk()
            .into_iter()
            .filter(|id| doc.effective_visible(*id))
            .filter_map(|id| doc.layer(id))
            .filter_map(|n| n.pixels().map(|buf| (buf, enabled_masks_of(doc, n))))
            .collect();
        composite_plane_frame_masked(&layers, frame)
    };
    Ok((w, h, plane))
}

/// Encode ONE layer's pixels as a deterministic PNG (smartobject.convert's
/// §11.5 asset carrier): premult storage → unpremul → sRGB 8-bit RGBA over
/// the content bounds, fixed encoder settings (same §8.2 posture as the
/// composite exporter — equal pixels, equal bytes).
pub fn encode_layer_png(
    doc: &crate::graph::document::Document,
    layer: crate::graph::id::LayerId,
) -> Result<Vec<u8>, EngineError> {
    let buf = doc
        .layer(layer)
        .and_then(|n| n.pixels())
        .ok_or_else(|| write_err("encoding layer PNG", "target is not a raster layer"))?;
    let b = buf.bounds();
    if b.is_empty() {
        return Err(write_err("encoding layer PNG", "layer has no content"));
    }
    let (w, h) = (b.width() as u32, b.height() as u32);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in b.min.y..b.max.y {
        for x in b.min.x..b.max.x {
            let px = buf.read_pixel_f32(IVec2::new(x, y));
            let a = px[3].clamp(0.0, 1.0);
            let inv = if a > 0.0 { 1.0 / a } else { 0.0 };
            // Interim storage posture: u8/u16 store sRGB-ENCODED values
            // (emit directly after unpremul); f32 stores LINEAR premult
            // (encode through the TRC — the §8.2 export convention).
            let f32_linear = buf.kind() == ChannelKind::F32;
            for &pc in px.iter().take(3) {
                let straight = (pc * inv).clamp(0.0, 1.0);
                let enc = if f32_linear {
                    srgb_encode(straight)
                } else {
                    straight
                };
                data.push((enc * 255.0).round() as u8);
            }
            data.push((a * 255.0).round() as u8);
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc
            .write_header()
            .map_err(|e| write_err("encoding layer PNG", e))?;
        wr.write_image_data(&data)
            .map_err(|e| write_err("encoding layer PNG", e))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::IRect;
    use crate::graph::id::ProfileId;

    // ============================================================
    // MAJOR-1 tile-major composite — BIT-EXACTNESS proof.
    //
    // The production walks (`composite_plane_frame`,
    // `composite_plane_frame_masked`) are now TILE-MAJOR. The load-bearing
    // property is BYTE-IDENTICAL output vs the old PIXEL-MAJOR walk. These
    // tests keep an independent pixel-major ORACLE (a verbatim copy of the
    // retired walk) and assert `f32::to_bits()` equality across the adversarial
    // corpus: multi-tile canvases, PARTIAL edge tiles, ABSENT tiles mid-canvas,
    // UNIFORM tiles, u8 + f32 storage, STRAIGHT + PREMULTIPLIED f32, α∈{0,1},
    // HDR super-brights, CROP frames (positive AND negative origin), sub-tile
    // canvases, and §11.2 MASKS (reveal/hide/density/inverted, incl. a masked
    // layer over absent + Uniform tiles).
    // ============================================================

    use crate::graph::id::MaskId;
    use crate::graph::mask_node::MaskNode;
    use crate::tile::model::ColorModelKind;

    /// splitmix64 — deterministic, no dependency.
    fn rng(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A layer with deterministic content over `rect`. `skip_tile` leaves one
    /// tile ABSENT mid-canvas; `uniform_at` installs a `Uniform` tile. Values
    /// span exact 0, exact 1, mids, and (for f32) HDR super-brights.
    fn build_layer(
        kind: ChannelKind,
        premul: bool,
        rect: IRect,
        seed: u64,
        skip_tile: Option<(i32, i32)>,
        uniform_at: Option<((i32, i32), [f32; 4])>,
    ) -> Buffer {
        let mut b = Buffer::new(kind, ColorModelKind::Rgba, ProfileId::nil());
        b.set_premultiplied_loaded(premul);
        let mut s = seed;
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                let (tx, ty) = (x.div_euclid(TILE_I32), y.div_euclid(TILE_I32));
                if skip_tile == Some((tx, ty)) {
                    continue;
                }
                if uniform_at.map(|(c, _)| c) == Some((tx, ty)) {
                    continue;
                }
                let r = rng(&mut s);
                let a = match (r >> 32) & 0x3 {
                    0 => 0.0,
                    1 => 1.0,
                    v => v as f32 / 3.0,
                };
                let q = |k: u32| -> f32 {
                    let straight = match (r >> (k * 8)) & 0x7 {
                        0 => 0.0,
                        1 => 1.0,
                        7 if kind == ChannelKind::F32 => 3.5, // HDR (§6.4)
                        v => v as f32 / 7.0,
                    };
                    if premul {
                        straight * a
                    } else {
                        straight
                    }
                };
                b.write_pixel_f32(IVec2::new(x, y), [q(0), q(1), q(2), a]);
            }
        }
        if let Some(((tx, ty), v)) = uniform_at {
            b.install_tile(
                TileCoord::new(0, tx, ty),
                std::sync::Arc::new(TileStorage::Uniform { value: v }),
            );
        }
        b.set_bounds_recorded(rect);
        b.recompute_bounds_pixel_exact();
        b
    }

    /// A Gray/f32 coverage mask over `rect` with a deterministic 0..1 ramp;
    /// `inverted`/`density` set the §2.6 props under test.
    fn build_mask(rect: IRect, seed: u64, inverted: bool, density: f32) -> MaskNode {
        let mut cov = Buffer::new(ChannelKind::F32, ColorModelKind::Gray, ProfileId::nil());
        let mut s = seed;
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                let r = rng(&mut s);
                let c = (r & 0xff) as f32 / 255.0; // spans 0, 1, and mids
                cov.write_pixel_f32(IVec2::new(x, y), [c, 0.0, 0.0, 0.0]);
            }
        }
        cov.set_bounds_recorded(rect);
        cov.recompute_bounds_pixel_exact();
        let mut m = MaskNode::new_raster(MaskId::from_uuid(uuid::Uuid::new_v4()), cov);
        m.inverted = inverted;
        m.density = density;
        m
    }

    /// The retired PIXEL-MAJOR walk, kept verbatim as an independent oracle.
    fn oracle_pixel_major(layers: &[(&Buffer, &[&MaskNode])], frame: IRect) -> Vec<f32> {
        // Resolve each stack ONCE, exactly as the production walk does —
        // the oracle's independence is in its PIXEL-MAJOR visit order, not
        // in re-deriving the §11.2 coverage formula.
        let resolved: Vec<crate::graph::mask_eval::ResolvedMasks<'_>> = layers
            .iter()
            .map(|(_, m)| crate::graph::mask_eval::ResolvedMasks::resolve(m))
            .collect();
        let (w, h) = (
            frame.width().max(0) as usize,
            frame.height().max(0) as usize,
        );
        let mut plane = Vec::with_capacity(w * h * 4);
        for y in frame.min.y..frame.max.y {
            for x in frame.min.x..frame.max.x {
                let p = IVec2::new(x, y);
                let mut acc = [0.0f32; 4];
                for ((buf, _), masks) in layers.iter().zip(resolved.iter()) {
                    let mut src = layer_pixel_linear(buf, p);
                    if !masks.is_empty() {
                        src = crate::apply_coverage(src, masks.coverage_at(p));
                    }
                    let ia = 1.0 - src[3];
                    for c in 0..4 {
                        acc[c] = src[c] + acc[c] * ia;
                    }
                }
                plane.extend_from_slice(&acc);
            }
        }
        plane
    }

    fn first_bit_divergence(a: &[f32], b: &[f32]) -> Option<usize> {
        assert_eq!(a.len(), b.len(), "plane length mismatch");
        a.iter()
            .zip(b)
            .position(|(x, y)| x.to_bits() != y.to_bits())
    }

    /// The core proof: the production tile-major `composite_plane_frame_masked`
    /// is BIT-IDENTICAL to the pixel-major oracle across the whole corpus and
    /// every frame (full canvas, positive + negative crop offsets, sub-tile).
    #[test]
    fn tiled_walk_is_bitexact_vs_pixel_major() {
        // 600×400 straddles a 3×2 tile grid with partial right/bottom tiles.
        let full = IRect::new(0, 0, 600, 400);
        let l0 = build_layer(ChannelKind::F32, true, full, 0xA1, None, None);
        let l1 = build_layer(ChannelKind::U8, true, full, 0xB2, Some((1, 0)), None);
        let l2 = build_layer(
            ChannelKind::F32,
            false, // STRAIGHT f32 — the ·a arm
            IRect::new(40, 30, 520, 300),
            0xC3,
            None,
            Some(((0, 1), [0.25, 0.5, 0.75, 0.5])), // a UNIFORM tile
        );
        let no_masks: &[&MaskNode] = &[];
        let sets: Vec<Vec<(&Buffer, &[&MaskNode])>> = vec![
            vec![(&l0, no_masks)],
            vec![(&l0, no_masks), (&l1, no_masks)],
            vec![(&l1, no_masks), (&l2, no_masks)],
            vec![(&l0, no_masks), (&l1, no_masks), (&l2, no_masks)],
        ];
        // Frames: full canvas, interior crop, a crop whose sample origin is
        // NEGATIVE (§19.1 buffers are unbounded — reads left/above 0 are real
        // all-zero content), and a crop crossing a tile boundary.
        let frames = [
            full,
            IRect::new(10, 20, 500, 380),
            IRect::new(-40, -10, 300, 260),
            IRect::new(250, 250, 600, 400),
        ];
        for (si, set) in sets.iter().enumerate() {
            for (fi, frame) in frames.iter().enumerate() {
                let want = oracle_pixel_major(set, *frame);
                // Exercise BOTH production doors: the masked one directly…
                let owned: Vec<(&Buffer, Vec<&MaskNode>)> =
                    set.iter().map(|(b, m)| (*b, m.to_vec())).collect();
                let got = composite_plane_frame_masked(&owned, *frame);
                assert_eq!(
                    first_bit_divergence(&want, &got),
                    None,
                    "masked door: set {si} frame {fi} diverged"
                );
                // …and the unmasked door (all sets here are unmasked).
                let bufs: Vec<&Buffer> = set.iter().map(|(b, _)| *b).collect();
                let got_unmasked = composite_plane_frame(&bufs, *frame);
                assert_eq!(
                    first_bit_divergence(&want, &got_unmasked),
                    None,
                    "unmasked door: set {si} frame {fi} diverged"
                );
            }
        }
    }

    // ============================================================
    // §5 OCCLUSION CULL (MASK-READBACK-PERF) — BIT-EXACTNESS proof.
    //
    // The production tile-major walk now starts each tile's layer loop at
    // `tile_occluder_base` under the `all_u8` gate. The independent oracle
    // (`oracle_pixel_major`) has NO cull. Every all-U8 set below must stay
    // byte-identical to the oracle — this is what makes the reorder safe.
    // ============================================================

    /// A FULLY-OPAQUE U8 layer over `rect` (alpha byte == 255 everywhere, so
    /// αs linearizes to exactly 1.0 — a valid occluder). `uniform_at` installs
    /// an opaque `Uniform` occluder tile; `hole` punches ONE non-opaque pixel
    /// (alpha 254/255) to defeat occlusion at that tile (dense-scan adversary).
    fn opaque_u8(
        rect: IRect,
        seed: u64,
        uniform_at: Option<(i32, i32)>,
        hole: Option<(i32, i32)>,
    ) -> Buffer {
        let mut b = Buffer::new(ChannelKind::U8, ColorModelKind::Rgba, ProfileId::nil());
        b.set_premultiplied_loaded(true);
        let mut s = seed;
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                let (tx, ty) = (x.div_euclid(TILE_I32), y.div_euclid(TILE_I32));
                if uniform_at == Some((tx, ty)) {
                    continue; // installed below
                }
                let r = rng(&mut s);
                // Opaque: alpha 1.0. u8 premul with a==1 ⇒ colors are their own
                // straight values, all in [0,1].
                let a = if hole == Some((x, y)) {
                    254.0 / 255.0
                } else {
                    1.0
                };
                let q = |k: u32| -> f32 {
                    let byte = ((r >> (k * 8)) & 0xff) as f32;
                    (byte / 255.0) * a
                };
                b.write_pixel_f32(IVec2::new(x, y), [q(0), q(1), q(2), a]);
            }
        }
        if let Some((tx, ty)) = uniform_at {
            b.install_tile(
                TileCoord::new(0, tx, ty),
                std::sync::Arc::new(TileStorage::Uniform {
                    value: [0.4, 0.6, 0.8, 1.0], // opaque
                }),
            );
        }
        b.set_bounds_recorded(rect);
        b.recompute_bounds_pixel_exact();
        b
    }

    /// Every all-U8 set — opaque occluders at top / middle / via Uniform tile,
    /// masked & partial & absent layers beneath, sub-opaque and masked
    /// NON-occluders on top, a dense occluder with a single non-opaque hole —
    /// stays byte-identical to the no-cull oracle across full/crop/negative/
    /// sub-tile frames. Proves the cull drops NO output byte.
    #[test]
    fn occlusion_cull_is_bitexact_vs_full_walk() {
        let full = IRect::new(0, 0, 600, 400);
        // Below layers: partial (varied α), absent-tile, and Uniform-tile U8.
        let base = build_layer(ChannelKind::U8, true, full, 0x11, None, None);
        let partial = build_layer(ChannelKind::U8, true, full, 0x22, Some((1, 0)), None);
        let partial2 = build_layer(
            ChannelKind::U8,
            true,
            IRect::new(20, 10, 560, 360),
            0x33,
            None,
            Some(((0, 1), [0.2, 0.4, 0.6, 0.5])), // partial Uniform tile
        );
        // Occluders (fully opaque, unmasked).
        let occ_full = opaque_u8(full, 0x44, None, None);
        let occ_uniform = opaque_u8(full, 0x55, Some((1, 1)), None);
        let occ_holed = opaque_u8(full, 0x66, None, Some((300, 200))); // one hole
                                                                       // A partial layer to sit ABOVE an occluder (composited normally).
        let over = build_layer(ChannelKind::U8, true, full, 0x77, None, None);

        // A masked layer whose BUFFER is fully opaque but which must NEVER be
        // treated as an occluder (§11.2 coverage is partial): defeats a naive
        // "alpha==1 ⇒ occluder" that ignores masks.
        let masked_opaque = opaque_u8(full, 0x88, None, None);
        let m0 = build_mask(full, 0x99, false, 1.0);
        let m0s: &[&MaskNode] = &[&m0];
        let no: &[&MaskNode] = &[];

        let sets: Vec<Vec<(&Buffer, &[&MaskNode])>> = vec![
            // 1. Opaque occluder on TOP over masked + partial + base ⇒ cull all.
            vec![
                (&base, no),
                (&partial, m0s),
                (&partial2, no),
                (&occ_full, no),
            ],
            // 2. Opaque occluder in the MIDDLE, partial ABOVE it.
            vec![(&base, no), (&occ_full, no), (&over, no)],
            // 3. Occluder via an opaque Uniform tile (O(1) occluder path).
            vec![(&base, m0s), (&partial, no), (&occ_uniform, no)],
            // 4. Dense occluder with ONE non-opaque hole ⇒ tiles with the hole
            //    must NOT cull (backdrop must survive at that tile).
            vec![(&base, no), (&partial, no), (&occ_holed, no)],
            // 5. Masked layer with an opaque buffer is NOT an occluder; the
            //    real occluder is the opaque layer above it.
            vec![(&base, no), (&masked_opaque, m0s), (&occ_full, no)],
            // 6. NO occluder anywhere (sub-opaque top) ⇒ base stays 0 (full walk).
            vec![(&base, no), (&partial, m0s), (&partial2, no)],
        ];
        let frames = [
            full,
            IRect::new(10, 20, 500, 380),
            IRect::new(-40, -10, 300, 260),
            IRect::new(250, 250, 600, 400),
            IRect::new(5, 5, 40, 40), // sub-tile
        ];
        for (si, set) in sets.iter().enumerate() {
            for (fi, frame) in frames.iter().enumerate() {
                let want = oracle_pixel_major(set, *frame);
                let owned: Vec<(&Buffer, Vec<&MaskNode>)> =
                    set.iter().map(|(b, m)| (*b, m.to_vec())).collect();
                let got = composite_plane_frame_masked(&owned, *frame);
                assert_eq!(
                    first_bit_divergence(&want, &got),
                    None,
                    "occlusion cull: set {si} frame {fi} diverged from the no-cull oracle"
                );
            }
        }
    }

    /// The `all_u8` gate EXCLUDES non-U8 stacks (F32/premul/HDR): even with a
    /// fully-opaque f32 occluder on top, the walk stays on the untouched path
    /// and is byte-identical to the oracle (the -0.0/NaN cases the cull refuses
    /// to reason about are simply never culled).
    #[test]
    fn occlusion_cull_excludes_non_u8_stacks() {
        let full = IRect::new(0, 0, 300, 200);
        // Fully-opaque f32 occluder (α==1 everywhere) atop f32 content.
        let mut occ = Buffer::new(ChannelKind::F32, ColorModelKind::Rgba, ProfileId::nil());
        occ.set_premultiplied_loaded(true);
        let mut s = 0xF00Du64;
        for y in full.min.y..full.max.y {
            for x in full.min.x..full.max.x {
                let r = rng(&mut s);
                let c = |k: u32| ((r >> (k * 8)) & 0xff) as f32 / 255.0;
                // Includes a deliberate -0.0 channel — the exact bit the cull's
                // proof forbids, so it must NOT engage on this stack.
                let ch0 = if (r & 1) == 0 { -0.0 } else { c(0) };
                occ.write_pixel_f32(IVec2::new(x, y), [ch0, c(1), c(2), 1.0]);
            }
        }
        occ.set_bounds_recorded(full);
        occ.recompute_bounds_pixel_exact();
        let below = build_layer(ChannelKind::F32, false, full, 0xBEEF, None, None);
        let no: &[&MaskNode] = &[];
        let set: Vec<(&Buffer, &[&MaskNode])> = vec![(&below, no), (&occ, no)];
        for frame in [full, IRect::new(-20, -20, 100, 100)] {
            let want = oracle_pixel_major(&set, frame);
            let owned: Vec<(&Buffer, Vec<&MaskNode>)> =
                set.iter().map(|(b, m)| (*b, m.to_vec())).collect();
            let got = composite_plane_frame_masked(&owned, frame);
            assert_eq!(
                first_bit_divergence(&want, &got),
                None,
                "non-U8 stack must stay on the exact path"
            );
        }
    }

    /// Sub-tile canvas + fully-disjoint (all-absent) layers stay bit-exact.
    #[test]
    fn tiled_walk_bitexact_subtile_and_disjoint() {
        let near = build_layer(
            ChannelKind::U8,
            true,
            IRect::new(0, 0, 37, 11),
            0xD4,
            None,
            None,
        );
        let far = build_layer(
            ChannelKind::F32,
            true,
            IRect::new(900, 900, 1000, 1000),
            0xE5,
            None,
            None,
        );
        let frame = IRect::new(0, 0, 37, 11);
        let none: &[&MaskNode] = &[];
        for set in [
            vec![(&near, none)],
            vec![(&far, none)],
            vec![(&near, none), (&far, none)],
            vec![(&far, none), (&near, none)],
        ] {
            let want = oracle_pixel_major(&set, frame);
            let owned: Vec<(&Buffer, Vec<&MaskNode>)> =
                set.iter().map(|(b, m)| (*b, m.to_vec())).collect();
            let got = composite_plane_frame_masked(&owned, frame);
            assert_eq!(
                first_bit_divergence(&want, &got),
                None,
                "sub-tile/disjoint diverged"
            );
        }
    }

    /// §11.2 MASKS: a masked layer (reveal-ramp, hide, density, inverted) over
    /// backdrops that include ABSENT and UNIFORM tiles stays bit-exact — the
    /// coverage multiply is applied per pixel identically to the oracle,
    /// including on the absent-tile pixels (where src is the [0,0,0,0] read).
    #[test]
    fn tiled_walk_bitexact_with_masks() {
        let full = IRect::new(0, 0, 300, 300);
        let bg = build_layer(ChannelKind::F32, true, full, 0x11, None, None);
        // Top layer with an absent tile and a uniform tile, masked.
        let top = build_layer(
            ChannelKind::U8,
            true,
            full,
            0x22,
            Some((1, 1)),
            Some(((0, 0), [0.3, 0.6, 0.9, 0.7])),
        );
        let m_ramp = build_mask(full, 0x33, false, 1.0);
        let m_dense = build_mask(full, 0x44, false, 0.4);
        let m_inv = build_mask(full, 0x55, true, 1.0);
        // A disabled mask must be the E=1 no-op (skipped by the resolver).
        let mut m_off = build_mask(full, 0x66, false, 1.0);
        m_off.enabled = false;

        let cases: Vec<Vec<&MaskNode>> = vec![
            vec![&m_ramp],
            vec![&m_dense],
            vec![&m_inv],
            vec![&m_ramp, &m_dense], // stacked product
            vec![&m_off, &m_ramp],   // disabled + enabled
        ];
        let frames = [
            full,
            IRect::new(20, 40, 280, 300),
            IRect::new(200, 10, 300, 290),
        ];
        for (ci, masks) in cases.iter().enumerate() {
            for (fi, frame) in frames.iter().enumerate() {
                let bare: &[&MaskNode] = &[];
                let set: Vec<(&Buffer, &[&MaskNode])> = vec![(&bg, bare), (&top, masks.as_slice())];
                let want = oracle_pixel_major(&set, *frame);
                let owned: Vec<(&Buffer, Vec<&MaskNode>)> =
                    vec![(&bg, Vec::new()), (&top, masks.clone())];
                let got = composite_plane_frame_masked(&owned, *frame);
                assert_eq!(
                    first_bit_divergence(&want, &got),
                    None,
                    "masked case {ci} frame {fi} diverged"
                );
            }
        }
    }

    fn tiny_png(pixels: &[[u8; 4]], w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(std::io::Cursor::new(&mut out), w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().unwrap();
            let data: Vec<u8> = pixels.iter().flatten().copied().collect();
            writer.write_image_data(&data).unwrap();
        }
        out
    }

    /// One-compositing-truth (O-UI filing, O-INT interim ruling): a
    /// visible TEXT layer walls the reference composite the SAME typed
    /// way on EVERY surface — export refuses exactly like readback
    /// (pre-fix, export SILENTLY OMITTED the text: the QA-F8 class).
    /// The wall clears when no un-servable content remains visible.
    #[test]
    fn text_layers_wall_the_reference_composite_typed_never_omitted() {
        let mut e =
            crate::engine::kinds::register(crate::engine::kind_impl::EngineAssembly::new()).build();
        e.apply_json(
            r#"{"schema":"appname.command.v1","id":"7e470000-0000-4000-8000-000000000001","kind":"doc.new","params":{"width":8,"height":8,"background":"white"}}"#,
        )
        .unwrap();
        let clean = export_png_bytes(&e).unwrap();
        // Test-door text node (the arm under test is the composite
        // gate, not text.add; core-only assemblies carry no TextLayout).
        let text_id = crate::graph::id::LayerId::from_uuid(uuid::Uuid::new_v4());
        {
            let doc = e.doc_mut_for_test();
            let root = doc.root();
            let n = doc.layer(root).map(|r| r.children().len()).unwrap();
            let data = crate::graph::node::TextLayerData::new(
                std::sync::Arc::new(b"{}".to_vec()),
                uuid::Uuid::new_v4(),
                IRect::new(0, 0, 4, 4),
            );
            doc.insert_child(
                root,
                n,
                crate::graph::node::LayerNode::new_text(text_id, "T", data),
            )
            .unwrap();
        }
        // BOTH doors refuse the SAME typed wall (not_implemented naming
        // the missing composite path) — no silent omission.
        for result in [
            export_png_bytes(&e).map(|_| ()),
            composite_plane_f32(&e).map(|_| ()),
        ] {
            let err = result.expect_err("text walls the reference composite");
            assert_eq!(err.code, ErrorCode::NotImplemented);
            // Composite-leg flip: on a SERVICE-LESS assembly the wall
            // now names the missing TextLayout service (the serve-pin
            // test below proves the serving arm with a probe service).
            assert!(
                err.message.contains("no TextLayout service"),
                "typed wall names the gap: {}",
                err.message
            );
        }
        // Change-detector pin (RETAPED per the O-TEXTVEC ruling:
        // TargetKindMismatch → NotImplemented — text is a sanctioned
        // parametric target; the CAPABILITY is what's missing): on this
        // service-less assembly rasterize answers typed NI; the real
        // serve (Door-6 page → tiles) flips this pin to a serve
        // assertion in the composite-leg commit.
        let err = e
            .apply_json(&format!(
                r#"{{"schema":"appname.command.v1","id":"7e470000-0000-4000-8000-000000000002","kind":"layer.rasterize","target":{{"layerId":"{}"}}}}"#,
                text_id.as_uuid()
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        // A HIDDEN text layer clears the wall (§4.6 step 1) — and the
        // composite equals the pre-insertion bytes (nothing omitted,
        // nothing fabricated).
        {
            let doc = e.doc_mut_for_test();
            let mut props = doc.props_block(text_id).unwrap();
            props.visible = false;
            doc.apply_props_block(text_id, &props).unwrap();
        }
        assert_eq!(export_png_bytes(&e).unwrap(), clean);
        // …and so does DELETING it (both halves of the claim pinned —
        // review D2): re-show first so the delete leg is the clearing
        // act, then layer.delete through the command door.
        {
            let doc = e.doc_mut_for_test();
            let mut props = doc.props_block(text_id).unwrap();
            props.visible = true;
            doc.apply_props_block(text_id, &props).unwrap();
        }
        assert!(export_png_bytes(&e).is_err(), "re-shown text re-walls");
        e.apply_json(&format!(
            r#"{{"schema":"appname.command.v1","id":"7e470000-0000-4000-8000-000000000003","kind":"layer.delete","target":{{"layerId":"{}"}}}}"#,
            text_id.as_uuid()
        ))
        .unwrap();
        assert_eq!(
            export_png_bytes(&e).unwrap(),
            clean,
            "deleted text clears the wall"
        );
    }

    /// §11.2 — the aud-replay **F2** pin, FLIPPED from a uniform refusal
    /// to a SERVE pin (O-RENDER v1.0.1). A raster LEAF mask is APPLIED on
    /// both reference doors (export and readback), never refused and never
    /// silently dropped:
    ///
    /// * a `hide_all` mask masks the layer OUT — the composite is
    ///   transparent, and crucially NOT the unmasked white bytes (the
    ///   exact anti-F2 proof: pre-fix the mask was dropped and the export
    ///   equalled the unmasked bytes; now the mask is genuinely applied);
    /// * a `reveal_all` mask is the §11.2 no-op (E = 1) — the composite
    ///   equals the unmasked bytes exactly;
    /// * a FEATHERED mask **serves** (Sub-slice B), on BOTH doors, and the
    ///   feather is really applied — the composite differs from the
    ///   unfeathered one. A serve that returned the unfeathered bytes
    ///   would pass a mere `is_ok()` check and be exactly the silent drop
    ///   QA-F8 forbids, so the difference is asserted, not the success.
    #[test]
    fn masks_serve_the_reference_composite_applied_never_dropped() {
        use crate::engine::kind_impl::EngineAssembly;
        let build = || {
            let mut e = crate::engine::kinds::register(EngineAssembly::new()).build();
            e.apply_json(
                r#"{"schema":"appname.command.v1","id":"f2000000-0000-4000-8000-000000000001","kind":"doc.new","params":{"width":8,"height":8,"background":"white"}}"#,
            )
            .unwrap();
            e
        };
        let bg_of = |e: &Engine| {
            let doc = e.document().unwrap();
            doc.children_of(doc.root()).unwrap()[0]
        };

        // Baseline: the UNMASKED single-raster doc (opaque white).
        let plain = build();
        let clean = export_png_bytes(&plain).unwrap();

        // Case 1 — hide_all mask: SERVES on both doors, and the mask is
        // APPLIED (the layer masks fully OUT ⇒ transparent), NEVER the
        // unmasked white bytes (the anti-F2 proof, now a serve).
        let mut e1 = build();
        let bg1 = bg_of(&e1);
        e1.apply_json(&format!(
            r#"{{"schema":"appname.command.v1","id":"f2000000-0000-4000-8000-000000000002","kind":"mask.add","params":{{"kind":"raster","newMaskId":"f2000ccc-0000-4000-8000-00000000000c","from":"hide_all"}},"target":{{"layerId":"{}"}}}}"#,
            bg1.as_uuid()
        ))
        .unwrap();
        // Both reference doors SERVE (no refusal).
        let hidden = export_png_bytes(&e1).expect("hide_all serves on export");
        let (_w, _h, plane) = composite_plane_f32(&e1).expect("hide_all serves on readback");
        assert!(
            plane.iter().all(|&v| v == 0.0),
            "hide_all masks the layer to transparent (E = 0 everywhere)"
        );
        assert_ne!(
            hidden, clean,
            "the mask is APPLIED, never dropped — hide_all can't equal the unmasked bytes"
        );

        // Case 2 — reveal_all mask: SERVES and is the §11.2 NO-OP (E = 1)
        // ⇒ byte-identical to the unmasked composite.
        let mut e2 = build();
        let bg2 = bg_of(&e2);
        e2.apply_json(&format!(
            r#"{{"schema":"appname.command.v1","id":"f2000000-0000-4000-8000-000000000003","kind":"mask.add","params":{{"kind":"raster","newMaskId":"f2000ddd-0000-4000-8000-00000000000d","from":"reveal_all"}},"target":{{"layerId":"{}"}}}}"#,
            bg2.as_uuid()
        ))
        .unwrap();
        assert_eq!(
            export_png_bytes(&e2).unwrap(),
            clean,
            "reveal_all is the §11.2 no-op — equals the unmasked composite"
        );

        // Case 3 — a FEATHERED mask SERVES on both doors (Sub-slice B),
        // and the feather is genuinely applied.
        //
        // The construct makes the signal unambiguous: the document is a
        // single OPAQUE WHITE 8×8 raster, so the composite pixel IS the
        // coverage `E` (premultiplied white × E = [E,E,E,E]). The mask is
        // `reveal_all`, whose stored coverage is exactly the 8×8 canvas —
        // a hard edge on all four sides, with absent (0) coverage beyond
        // it. Feathering by 8 px (σ = 8/3, Rmask = ⌈8⌉ = 8) therefore
        // reaches every pixel of the canvas, so EVERY value must drop
        // below 1.0. These are derived from the normalized kernel and the
        // §2.4 absent-reads-0 rule, not fitted to the output.
        let mut e3 = build();
        let bg3 = bg_of(&e3);
        e3.apply_json(&format!(
            r#"{{"schema":"appname.command.v1","id":"f2000000-0000-4000-8000-000000000004","kind":"mask.add","params":{{"kind":"raster","newMaskId":"f2000eee-0000-4000-8000-00000000000e","from":"reveal_all"}},"target":{{"layerId":"{}"}}}}"#,
            bg3.as_uuid()
        ))
        .unwrap();
        e3.apply_json(&format!(
            r#"{{"schema":"appname.command.v1","id":"f2000000-0000-4000-8000-000000000005","kind":"mask.set_props","target":{{"layerId":"{}","maskId":"f2000eee-0000-4000-8000-00000000000e"}},"params":{{"feather":8.0}}}}"#,
            bg3.as_uuid()
        ))
        .unwrap();
        // BOTH doors serve (they used to refuse typed).
        let feathered_png = export_png_bytes(&e3).expect("feather serves on export");
        let (fw, fh, feathered) = composite_plane_f32(&e3).expect("feather serves on readback");
        assert_eq!((fw, fh), (8, 8));

        // ...and the feather is APPLIED, not dropped: the unfeathered
        // `reveal_all` composite is the unmasked white bytes (case 2), so
        // equal bytes here would mean the Gaussian never ran.
        assert_ne!(
            feathered_png, clean,
            "the feather is APPLIED — a feathered reveal_all cannot equal the \
             unmasked bytes (that is the silent drop QA-F8 forbids)"
        );

        // Every pixel is strictly inside (0, 1): Rmask = 8 covers the whole
        // 8×8 canvas, so no pixel keeps full coverage, and the kernel is
        // normalized over a non-negative field that is 1 somewhere, so none
        // reaches 0 either.
        let at = |x: usize, y: usize| feathered[(y * 8 + x) * 4];
        for y in 0..8 {
            for x in 0..8 {
                let v = at(x, y);
                assert!(
                    v > 0.0 && v < 1.0,
                    "every pixel is inside the feather ramp at ({x},{y}): {v}"
                );
                // Opaque white × E is premultiplied-uniform: all four
                // channels carry the same E, so a change to the coverage
                // multiply cannot quietly reinterpret this scan.
                for c in 1..4 {
                    assert_eq!(feathered[(y * 8 + x) * 4 + c], v, "lane {c} at ({x},{y})");
                }
            }
        }
        // The corner sees absent coverage on TWO sides, the center on none
        // within its 8 px reach that the corner does not also see — so the
        // ramp rises inward, strictly.
        assert!(
            at(0, 0) < at(3, 3),
            "the feather ramp rises inward: corner {} vs centre {}",
            at(0, 0),
            at(3, 3)
        );
        // Symmetry: the construct and the kernel are both 4-fold symmetric
        // about the canvas centre, so the four corners must agree exactly.
        assert_eq!(at(0, 0), at(7, 0));
        assert_eq!(at(0, 0), at(0, 7));
        assert_eq!(at(0, 0), at(7, 7));
    }

    /// Hidden-group ruling (render correct, CPU-reference bug): a leaf
    /// composites only if it is own-visible AND every ANCESTOR group is
    /// visible. The reference export must EXCLUDE an own-visible raster leaf
    /// that lives inside a HIDDEN group — matching the render compositor,
    /// which skips a hidden group's whole subtree (§4.6/§4.7). Pre-fix the
    /// raster arm filtered by own `visible()` only and wrongly composited
    /// the leaf into the export → export-vs-preview divergence.
    #[test]
    fn hidden_group_excludes_its_own_visible_leaf_from_the_reference_export() {
        use crate::engine::kind_impl::EngineAssembly;

        const RED: &str = "aa000000-0000-4000-8000-000000000001";
        const G: &str = "aa000000-0000-4000-8000-000000000002";
        const G2: &str = "aa000000-0000-4000-8000-000000000003";

        // A 4×4 white document (doc.new lays down an opaque white bg raster).
        let build = || {
            let mut e = crate::engine::kinds::register(EngineAssembly::new()).build();
            e.apply_json(
                r#"{"schema":"appname.command.v1","id":"aa000000-0000-4000-8000-0000000000ff","kind":"doc.new","params":{"width":4,"height":4,"background":"white"}}"#,
            )
            .unwrap();
            e
        };
        // One journaled command with a unique id (n) — params is a raw JSON
        // object; an optional layer target rides after it.
        let cmd = |e: &mut Engine, n: u32, kind: &str, target: Option<&str>, params: &str| {
            let tgt = match target {
                Some(t) => format!(r#","target":{{"layerId":"{t}"}}"#),
                None => String::new(),
            };
            e.apply_json(&format!(
                r#"{{"schema":"appname.command.v1","id":"aa000000-0000-4000-8000-{n:012x}","kind":"{kind}","params":{params}{tgt}}}"#
            ))
            .unwrap();
        };
        // Paste an opaque RED 4×4 leaf on top of the white bg — a real,
        // content-bearing own-visible leaf (the thing excluded when its
        // enclosing group is hidden). Opaque + full-canvas ⇒ when it
        // composites the export is pure RED; when excluded it is pure WHITE.
        let paste_red = |e: &mut Engine| {
            let asset = e
                .add_raster_asset(tiny_png(&[[255, 0, 0, 255]; 16], 4, 4))
                .unwrap();
            cmd(
                e,
                0x10,
                "edit.paste",
                None,
                &format!(r#"{{"assetId":"{asset}","at":[0,0],"newLayerId":"{RED}"}}"#),
            );
        };

        // The two poles: WHITE (leaf absent/excluded) vs RED (leaf shown).
        let white = export_png_bytes(&build()).unwrap();
        let red = {
            let mut e = build();
            paste_red(&mut e);
            export_png_bytes(&e).unwrap()
        };
        assert_ne!(
            white, red,
            "the two poles must differ (test would be vacuous)"
        );

        // (1) HIDDEN group with an OWN-VISIBLE leaf ⇒ leaf EXCLUDED == WHITE.
        let mut e1 = build();
        paste_red(&mut e1);
        cmd(
            &mut e1,
            0x20,
            "layer.group",
            None,
            &format!(r#"{{"layerIds":["{RED}"],"newGroupId":"{G}"}}"#),
        );
        cmd(
            &mut e1,
            0x30,
            "layer.set_props",
            Some(G),
            r#"{"visible":false}"#,
        );
        assert_eq!(
            export_png_bytes(&e1).unwrap(),
            white,
            "an own-visible leaf inside a HIDDEN group must be excluded (render skips it)"
        );

        // (2) NESTED hidden groups: hidden G ⊃ visible G2 ⊃ visible RED ⇒
        // still EXCLUDED (a hidden ancestor anywhere up the chain wins).
        let mut e2 = build();
        paste_red(&mut e2);
        cmd(
            &mut e2,
            0x20,
            "layer.group",
            None,
            &format!(r#"{{"layerIds":["{RED}"],"newGroupId":"{G2}"}}"#),
        );
        cmd(
            &mut e2,
            0x21,
            "layer.group",
            None,
            &format!(r#"{{"layerIds":["{G2}"],"newGroupId":"{G}"}}"#),
        );
        cmd(
            &mut e2,
            0x30,
            "layer.set_props",
            Some(G),
            r#"{"visible":false}"#,
        );
        assert_eq!(
            export_png_bytes(&e2).unwrap(),
            white,
            "a hidden OUTER group excludes an own-visible leaf nested below a visible inner group"
        );

        // (3) VISIBLE group with a HIDDEN leaf ⇒ excluded (own visibility —
        // already correct pre-fix; this must NOT change).
        let mut e3 = build();
        paste_red(&mut e3);
        cmd(
            &mut e3,
            0x20,
            "layer.group",
            None,
            &format!(r#"{{"layerIds":["{RED}"],"newGroupId":"{G}"}}"#),
        );
        cmd(
            &mut e3,
            0x30,
            "layer.set_props",
            Some(RED),
            r#"{"visible":false}"#,
        );
        assert_eq!(
            export_png_bytes(&e3).unwrap(),
            white,
            "a hidden leaf in a visible group stays excluded (unchanged behavior)"
        );

        // (4) HIDDEN group with a HIDDEN leaf ⇒ excluded.
        let mut e4 = build();
        paste_red(&mut e4);
        cmd(
            &mut e4,
            0x20,
            "layer.group",
            None,
            &format!(r#"{{"layerIds":["{RED}"],"newGroupId":"{G}"}}"#),
        );
        cmd(
            &mut e4,
            0x30,
            "layer.set_props",
            Some(G),
            r#"{"visible":false}"#,
        );
        cmd(
            &mut e4,
            0x31,
            "layer.set_props",
            Some(RED),
            r#"{"visible":false}"#,
        );
        assert_eq!(
            export_png_bytes(&e4).unwrap(),
            white,
            "hidden group + hidden leaf: excluded"
        );

        // (5) CONTROL — VISIBLE group with a VISIBLE leaf ⇒ the leaf SHOWS
        // (== RED). Proves the fix does not OVER-exclude effectively-visible
        // leaves that merely happen to sit inside a (visible) group.
        let mut e5 = build();
        paste_red(&mut e5);
        cmd(
            &mut e5,
            0x20,
            "layer.group",
            None,
            &format!(r#"{{"layerIds":["{RED}"],"newGroupId":"{G}"}}"#),
        );
        assert_eq!(
            export_png_bytes(&e5).unwrap(),
            red,
            "an own-visible leaf in a VISIBLE group still composites (no over-exclusion)"
        );
    }

    /// Deterministic Door-6 probe for the SERVE pins: a fixed 4×4 page at
    /// (2, 2)–(6, 6), premultiplied linear half-opaque red, with a call
    /// counter (the replay/value-capture pin). Every other surface answers
    /// typed — the probe never claims layout capability beyond compose.
    struct PageProbe {
        calls: std::sync::atomic::AtomicUsize,
    }
    impl PageProbe {
        fn page() -> crate::engine::text_ops::TextPageData {
            crate::engine::text_ops::TextPageData {
                rect: IRect::new(2, 2, 6, 6),
                pixels: vec![[0.4, 0.0, 0.0, 0.5]; 16],
            }
        }
    }
    impl crate::engine::text_ops::TextLayout for PageProbe {
        fn schema(&self) -> &'static str {
            crate::engine::text_ops::TEXT_SIDECAR_SCHEMA
        }
        fn validate_sidecar(
            &self,
            _bytes: &[u8],
        ) -> Result<crate::engine::text_ops::TextLayoutResult, EngineError> {
            Err(EngineError::bad_target("probe: compose-only"))
        }
        fn sidecar_from_add_params(
            &self,
            _params: &serde_json::Map<String, serde_json::Value>,
        ) -> Result<crate::engine::text_ops::TextLayoutResult, EngineError> {
            // Enough for a JOURNALED text.add (the replay pin needs the
            // whole sequence in the journal).
            Ok(crate::engine::text_ops::TextLayoutResult {
                sidecar: b"{}".to_vec(),
                bounds: IRect::new(2, 2, 6, 6),
            })
        }
        fn apply_edit(
            &self,
            _current: &[u8],
            _params: &serde_json::Map<String, serde_json::Value>,
        ) -> Result<crate::engine::text_ops::TextLayoutResult, EngineError> {
            Err(EngineError::bad_target("probe: compose-only"))
        }
        fn apply_style_range(
            &self,
            _current: &[u8],
            _range: (u64, u64),
            _style: &serde_json::Map<String, serde_json::Value>,
        ) -> Result<crate::engine::text_ops::TextLayoutResult, EngineError> {
            Err(EngineError::bad_target("probe: compose-only"))
        }
        fn apply_paragraph_range(
            &self,
            _current: &[u8],
            _range: (u64, u64),
            _paragraph: &serde_json::Map<String, serde_json::Value>,
        ) -> Result<crate::engine::text_ops::TextLayoutResult, EngineError> {
            Err(EngineError::bad_target("probe: compose-only"))
        }
        fn content_len(&self, _sidecar: &[u8]) -> Result<u64, EngineError> {
            Err(EngineError::bad_target("probe: compose-only"))
        }
        fn layout_and_compose(
            &self,
            _sidecar: &[u8],
        ) -> Result<
            (
                crate::engine::text_ops::TextPageData,
                crate::engine::text_ops::LaidMetrics,
            ),
            EngineError,
        > {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok((
                Self::page(),
                crate::engine::text_ops::LaidMetrics {
                    bounds: IRect::new(2, 2, 6, 6),
                    line_count: 1,
                    overflowed: false,
                },
            ))
        }
        fn outline_geometry(&self, _sidecar: &[u8]) -> Result<Vec<u8>, EngineError> {
            Err(EngineError::bad_target("probe: compose-only"))
        }
        fn caret_geometry(
            &self,
            _sidecar: &[u8],
            _scalar_index: u64,
        ) -> Result<Vec<crate::engine::text_ops::CaretRect>, EngineError> {
            Err(EngineError::bad_target("probe: compose-only"))
        }
        fn hit_test(&self, _sidecar: &[u8], _point: [f64; 2]) -> Result<u64, EngineError> {
            Err(EngineError::bad_target("probe: compose-only"))
        }
        fn register_embedded_face(
            &self,
            _asset: uuid::Uuid,
            _bytes: &[u8],
        ) -> Result<(), EngineError> {
            Ok(())
        }
        fn enumerate_faces(&self) -> Vec<crate::engine::text_ops::FontFaceInfo> {
            Vec::new()
        }
        fn fonts_stamp(&self) -> u64 {
            1
        }
    }

    fn probe_engine() -> (Engine, std::sync::Arc<PageProbe>) {
        let probe = std::sync::Arc::new(PageProbe {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let svc: std::sync::Arc<dyn crate::engine::text_ops::TextLayout> = probe.clone();
        let mut e = crate::engine::kinds::register(crate::engine::kind_impl::EngineAssembly::new())
            .with_text_layout(svc)
            .build();
        e.apply_json(
            r#"{"schema":"appname.command.v1","id":"7e470000-0000-4000-8000-000000000010","kind":"doc.new","params":{"width":8,"height":8,"background":"white","depth":"f32"}}"#,
        )
        .unwrap();
        (e, probe)
    }

    fn insert_probe_text(e: &mut Engine) -> crate::graph::id::LayerId {
        let text_id = crate::graph::id::LayerId::from_uuid(uuid::Uuid::new_v4());
        let doc = e.doc_mut_for_test();
        let root = doc.root();
        let n = doc.layer(root).map(|r| r.children().len()).unwrap();
        let data = crate::graph::node::TextLayerData::new(
            std::sync::Arc::new(b"{}".to_vec()),
            uuid::Uuid::new_v4(),
            IRect::new(2, 2, 6, 6),
        );
        doc.insert_child(
            root,
            n,
            crate::graph::node::LayerNode::new_text(text_id, "T", data),
        )
        .unwrap();
        text_id
    }

    /// The SERVE pin (the wall LIFTS): with a TextLayout service on the
    /// assembly, export and readback SERVE identically — the page
    /// composites source-over into the reference plane, byte-equal
    /// across both doors, hand-math-checked.
    #[test]
    fn text_page_serves_identically_through_export_and_readback() {
        let (mut e, probe) = probe_engine();
        insert_probe_text(&mut e);
        let (w, h, plane) = composite_plane_f32(&e).unwrap();
        assert_eq!((w, h), (8, 8));
        // Outside the page: white backdrop untouched.
        assert_eq!(&plane[0..4], &[1.0, 1.0, 1.0, 1.0]);
        // Inside the page: over(white, [0.4, 0, 0, 0.5]) =
        // s + d·(1−αs) = [0.4+0.5, 0+0.5, 0+0.5, 0.5+0.5].
        let i = (3 * 8 + 3) * 4;
        assert_eq!(&plane[i..i + 4], &[0.9, 0.5, 0.5, 1.0], "§4.4 over math");
        // One truth: the exporter is exactly this plane + the quantizer.
        assert_eq!(
            export_png_bytes(&e).unwrap(),
            encode_composite_png(w, h, &plane).unwrap()
        );
        // The read door ran compose twice (once per composite call) —
        // the READ path is uncaptured by design (no envelope; §23.2
        // purity in (sidecar, fonts) — posture recorded in the log).
        assert_eq!(probe.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    /// The rasterize SERVE + the D-48 value-capture pin: the page bakes
    /// into a raster layer (class-C snapshot undo), the capture rides the
    /// journal, and REPLAY consumes it — the probe's compose counter
    /// proves layout is never re-run. The text layer arrives via a
    /// JOURNALED text.add (probe-served) so the replay leg is complete.
    #[test]
    fn text_rasterize_serves_and_replay_consumes_the_capture() {
        let (mut e, probe) = probe_engine();
        let text_id = crate::graph::id::LayerId::from_uuid(
            uuid::Uuid::parse_str("7e470000-aaaa-4000-8000-000000000001").unwrap(),
        );
        e.apply_json(&format!(
            r#"{{"schema":"appname.command.v1","id":"7e470000-0000-4000-8000-000000000012","kind":"text.add","params":{{"layout":"point","origin":[2,2],"newLayerId":"{}"}}}}"#,
            text_id.as_uuid()
        ))
        .unwrap();
        let before = e.state_hash();
        e.apply_json(&format!(
            r#"{{"schema":"appname.command.v1","id":"7e470000-0000-4000-8000-000000000011","kind":"layer.rasterize","target":{{"layerId":"{}"}}}}"#,
            text_id.as_uuid()
        ))
        .unwrap();
        let calls_after_apply = probe.calls.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(calls_after_apply, 1, "apply composed once");
        {
            let doc = e.document().unwrap();
            let node = doc.layer(text_id).unwrap();
            assert!(node.text_data().is_none(), "kind swapped");
            let px = node.pixels().expect("raster now");
            assert_eq!(
                px.read_pixel_f32(IVec2::new(3, 3)),
                [0.4, 0.0, 0.0, 0.5],
                "page baked premul-verbatim at f32"
            );
            assert!(px.validate_premultiplied().is_ok());
        }
        // Class-C snapshot: undo restores the TEXT node wholesale.
        let after = e.state_hash();
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        assert!(e
            .document()
            .unwrap()
            .layer(text_id)
            .unwrap()
            .text_data()
            .is_some());
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);
        assert_eq!(
            probe.calls.load(std::sync::atomic::Ordering::SeqCst),
            calls_after_apply,
            "undo/redo are snapshot swaps — no re-compose"
        );
        // REPLAY (fresh engine, same assembly + probe): the journal
        // carries the capture; the arm consumes it — compose NOT re-run.
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
        assert_eq!(
            probe.calls.load(std::sync::atomic::Ordering::SeqCst),
            calls_after_apply,
            "D-48: replay consumed the page capture, layout never re-ran"
        );
    }

    /// Coverage-only Door-1 probe for the SHAPE serve pins: uniform 0.5
    /// coverage over (2, 2)–(6, 6); every other surface typed.
    struct CoverageProbe;
    impl crate::engine::vector_ops::VectorGeometry for CoverageProbe {
        fn schema(&self) -> &'static str {
            crate::engine::vector_ops::VECTOR_SIDECAR_SCHEMA
        }
        fn validate_sidecar(
            &self,
            _bytes: &[u8],
        ) -> Result<crate::engine::vector_ops::GeometryResult, EngineError> {
            Err(EngineError::bad_target("probe: coverage-only"))
        }
        fn path_param_sidecar(
            &self,
            _path: &serde_json::Map<String, serde_json::Value>,
        ) -> Result<crate::engine::vector_ops::GeometryResult, EngineError> {
            Err(EngineError::bad_target("probe: coverage-only"))
        }
        fn primitive_sidecar(
            &self,
            _kind: &str,
            _primitive: &serde_json::Map<String, serde_json::Value>,
        ) -> Result<crate::engine::vector_ops::GeometryResult, EngineError> {
            Err(EngineError::bad_target("probe: coverage-only"))
        }
        fn path_boolean(
            &self,
            _a: &[u8],
            _b: &[u8],
            _op: crate::engine::vector_ops::VectorBoolOp,
        ) -> Result<crate::engine::vector_ops::GeometryResult, EngineError> {
            Err(EngineError::bad_target("probe: coverage-only"))
        }
        fn rasterize_fill(
            &self,
            _sidecar: &[u8],
        ) -> Result<crate::engine::vector_ops::CoveragePlaneData, EngineError> {
            Ok(crate::engine::vector_ops::CoveragePlaneData {
                rect: IRect::new(2, 2, 6, 6),
                values: vec![0.5; 16],
            })
        }
        /// A 2 × 2 band at (1,1)-(3,3): cell (2,2) OVERLAPS the fill rect
        /// (2,2)-(6,6) — the paint-order discriminator — and cell (1,1)
        /// sits outside it, so the union rect really is a union. Coverage
        /// rides `style.width` so a style that never reached the door
        /// cannot pass unnoticed.
        fn shape_stroke_coverage(
            &self,
            _sidecar: &[u8],
            style: crate::engine::vector_ops::ShapeStrokeStyle,
        ) -> Result<crate::engine::vector_ops::CoveragePlaneData, EngineError> {
            Ok(crate::engine::vector_ops::CoveragePlaneData {
                rect: IRect::new(1, 1, 3, 3),
                values: vec![(style.width as f32 / 4.0).clamp(0.0, 1.0); 4],
            })
        }
        fn apply_edit_ops(
            &self,
            _sidecar: &[u8],
            _ops: &[serde_json::Value],
        ) -> Result<crate::engine::vector_ops::GeometryResult, EngineError> {
            Err(EngineError::bad_target("probe: coverage-only"))
        }
    }

    /// PROBE seam fns (tests ONLY — the production injection is
    /// appname-text's export via the assembly one-liner; these define the
    /// TEST expectation for core's plumbing, the C4 hand-oracle pattern).
    /// Payload-sensitive so the STROKE leg's paint is distinguishable from
    /// the FILL leg's: a `color.comps` payload rides through verbatim, and
    /// a payload WITHOUT one keeps the original probe constant — every
    /// expectation written before the stroke leg landed is untouched.
    fn probe_fill(fill: Option<&serde_json::Value>) -> [f32; 4] {
        let comps = fill
            .and_then(|f| f.get("color"))
            .and_then(|c| c.get("comps"))
            .and_then(serde_json::Value::as_array)
            .filter(|a| a.len() == 4);
        match comps {
            Some(a) => {
                let mut out = [0.0f32; 4];
                for (o, v) in out.iter_mut().zip(a) {
                    *o = v.as_f64().unwrap_or(0.0) as f32;
                }
                out
            }
            None => [0.25, 0.5, 0.75, 1.0],
        }
    }
    fn probe_premul(rgba: [f32; 4], coverage: f32) -> [f32; 4] {
        [
            rgba[0] * rgba[3] * coverage,
            rgba[1] * rgba[3] * coverage,
            rgba[2] * rgba[3] * coverage,
            rgba[3] * coverage,
        ]
    }

    fn insert_probe_shape(e: &mut Engine, fill: serde_json::Value, stroke: serde_json::Value) {
        let pid = crate::graph::id::PathId::from_uuid(uuid::Uuid::new_v4());
        let sid = crate::graph::id::LayerId::from_uuid(uuid::Uuid::new_v4());
        let doc = e.doc_mut_for_test();
        let sidecar = std::sync::Arc::new(b"{}".to_vec());
        let hash = uuid::Uuid::new_v4();
        doc.insert_path(crate::graph::path_node::PathNode::new(
            pid,
            None,
            sidecar,
            hash,
            IRect::new(2, 2, 6, 6),
        ))
        .unwrap();
        let root = doc.root();
        let n = doc.layer(root).map(|r| r.children().len()).unwrap();
        let data = crate::graph::node::ShapeLayerData {
            path: pid,
            fill,
            stroke,
            bool_ops: Vec::new(),
        };
        doc.insert_child(
            root,
            n,
            crate::graph::node::LayerNode::new_shape(sid, "S", data),
        )
        .unwrap();
    }

    /// The SHAPE serve pin (the two-door convention): Door-1 coverage ×
    /// the layer's FillContent through the seam — hand-math-checked and
    /// byte-equal across both composite doors; the gates (stroke,
    /// non-solid fill, missing seam) refuse typed.
    #[test]
    fn shape_coverage_times_fill_serves_and_gates_typed() {
        use serde_json::json;
        let assembly = || {
            crate::engine::kinds::register(crate::engine::kind_impl::EngineAssembly::new())
                .with_vector_geometry(std::sync::Arc::new(CoverageProbe))
                .with_fill_convert(crate::engine::kind_impl::FillConvertRouting {
                    fill_to_linear_rgba: probe_fill,
                    premultiply_coverage: probe_premul,
                })
        };
        let mut e = assembly().build();
        e.apply_json(
            r#"{"schema":"appname.command.v1","id":"7e470000-0000-4000-8000-000000000020","kind":"doc.new","params":{"width":8,"height":8,"background":"white","depth":"f32"}}"#,
        )
        .unwrap();
        insert_probe_shape(&mut e, json!({"type": "solid"}), serde_json::Value::Null);
        let (w, h, plane) = composite_plane_f32(&e).unwrap();
        // over(white, premul([0.25, 0.5, 0.75, 1], 0.5)):
        // s = [0.125, 0.25, 0.375, 0.5]; d·(1−αs) = 0.5 each.
        let i = (3 * 8 + 3) * 4;
        assert_eq!(
            &plane[i..i + 4],
            &[0.625, 0.75, 0.875, 1.0],
            "coverage × fill over white (§4.4 + the seam product)"
        );
        assert_eq!(&plane[0..4], &[1.0, 1.0, 1.0, 1.0], "outside untouched");
        assert_eq!(
            export_png_bytes(&e).unwrap(),
            encode_composite_png(w, h, &plane).unwrap(),
            "one truth across both doors"
        );
        // A ZERO-WIDTH stroke (the contract's shape_add default) is NO
        // visible stroke — it SERVES identically (gating on non-null
        // would wall every production shape).
        let mut ez = assembly().build();
        ez.apply_json(
            r#"{"schema":"appname.command.v1","id":"7e470000-0000-4000-8000-00000000002f","kind":"doc.new","params":{"width":8,"height":8,"background":"white","depth":"f32"}}"#,
        )
        .unwrap();
        insert_probe_shape(&mut ez, json!({"type": "solid"}), json!({"widthPx": 0}));
        assert_eq!(
            composite_plane_f32(&ez).unwrap().2[i..i + 4],
            [0.625, 0.75, 0.875, 1.0],
            "zero-width stroke serves (contract shape_add default)"
        );
        // Gates, each typed: gradient fill (the §22 gap), VISIBLE
        // stroke, missing fill seam.
        let mut e2 = assembly().build();
        e2.apply_json(
            r#"{"schema":"appname.command.v1","id":"7e470000-0000-4000-8000-000000000021","kind":"doc.new","params":{"width":8,"height":8}}"#,
        )
        .unwrap();
        insert_probe_shape(
            &mut e2,
            json!({"type": "gradient"}),
            serde_json::Value::Null,
        );
        let err = composite_plane_f32(&e2).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert!(err.message.contains("§22"), "{}", err.message);

        // THE FLIP (Tier-1 item 4): a VISIBLE stroke no longer walls — it
        // composites over the fill INSIDE the layer's own page. Probe
        // width 2 ⇒ band coverage 0.5, paint opaque red:
        //   stroke premul = [0.5, 0, 0, 0.5];
        //   fill   premul = [0.125, 0.25, 0.375, 0.5]  (as above);
        //   stroke OVER fill at the overlap cell (2,2), inv = 0.5 ⇒
        //     [0.5625, 0.125, 0.1875, 0.75];
        //   that over the WHITE ground (inv = 0.25) ⇒
        //     [0.8125, 0.375, 0.4375, 1.0].
        let mut e3 = assembly().build();
        e3.apply_json(
            r#"{"schema":"appname.command.v1","id":"7e470000-0000-4000-8000-000000000022","kind":"doc.new","params":{"width":8,"height":8,"background":"white","depth":"f32"}}"#,
        )
        .unwrap();
        insert_probe_shape(
            &mut e3,
            json!({"type": "solid"}),
            json!({"widthPx": 2, "color": {"model": "rgba", "comps": [1.0, 0.0, 0.0, 1.0]}}),
        );
        let (_, _, stroked) = composite_plane_f32(&e3).unwrap();
        let at = |x: usize, y: usize| {
            let k = (y * 8 + x) * 4;
            [stroked[k], stroked[k + 1], stroked[k + 2], stroked[k + 3]]
        };
        assert_eq!(
            at(2, 2),
            [0.8125, 0.375, 0.4375, 1.0],
            "stroke OVER fill at the overlap cell (§20.2 paint order)"
        );
        // Outside the fill entirely: stroke over the ground alone,
        // [0.5, 0, 0, 0.5] over white (inv 0.5) ⇒ [1.0, 0.5, 0.5, 1.0].
        assert_eq!(
            at(1, 1),
            [1.0, 0.5, 0.5, 1.0],
            "the band reaches past the fill rect"
        );
        // Fill-only cell: byte-for-byte the pre-stroke answer.
        assert_eq!(
            at(3, 3),
            [0.625, 0.75, 0.875, 1.0],
            "fill-only cell untouched"
        );
        assert_eq!(
            export_png_bytes(&e3).unwrap(),
            encode_composite_png(8, 8, &stroked).unwrap(),
            "one truth across both doors, stroked"
        );

        // The §20.2 arms this slice does NOT compute still refuse typed,
        // each naming its own rule — refused, never omitted (QA-F8).
        for (label, stroke, needle) in [
            (
                "dashes",
                json!({"widthPx": 2, "dashes": [4, 2]}),
                "§20.2 rule-4",
            ),
            (
                "profile",
                json!({"widthPx": 2, "profile": [[0, 1], [1, 0.5]]}),
                "§20.2 rule-6",
            ),
            (
                "gradient paint",
                json!({"widthPx": 2, "fill": {"type": "gradient"}}),
                "§22",
            ),
        ] {
            let mut eg = assembly().build();
            eg.apply_json(
                r#"{"schema":"appname.command.v1","id":"7e470000-0000-4000-8000-000000000024","kind":"doc.new","params":{"width":8,"height":8}}"#,
            )
            .unwrap();
            insert_probe_shape(&mut eg, json!({"type": "solid"}), stroke);
            let err = composite_plane_f32(&eg).unwrap_err();
            assert_eq!(err.code, ErrorCode::NotImplemented, "{label}");
            assert!(err.message.contains(needle), "{label}: {}", err.message);
        }

        let mut e4 =
            crate::engine::kinds::register(crate::engine::kind_impl::EngineAssembly::new())
                .with_vector_geometry(std::sync::Arc::new(CoverageProbe))
                .build();
        e4.apply_json(
            r#"{"schema":"appname.command.v1","id":"7e470000-0000-4000-8000-000000000023","kind":"doc.new","params":{"width":8,"height":8}}"#,
        )
        .unwrap();
        insert_probe_shape(&mut e4, json!({"type": "solid"}), serde_json::Value::Null);
        let err = composite_plane_f32(&e4).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert!(err.message.contains("fill-math seam"), "{}", err.message);
    }

    /// O-IO door 4: the f32 composite readback door serves the SAME plane
    /// the PNG exporter encodes (one math path — no 8-bit round trip).
    #[test]
    fn composite_plane_f32_matches_the_png_export_math() {
        let mut e =
            crate::engine::kinds::register(crate::engine::kind_impl::EngineAssembly::new()).build();
        e.apply_json(
            r#"{"schema":"appname.command.v1","id":"d004aaaa-0000-4000-8000-000000000001","kind":"doc.new","params":{"width":8,"height":8,"background":"white"}}"#,
        )
        .unwrap();
        let (w, h, plane) = composite_plane_f32(&e).unwrap();
        assert_eq!((w, h), (8, 8));
        assert_eq!(plane.len(), 8 * 8 * 4);
        // White background: linear premultiplied [1, 1, 1, 1] everywhere.
        for px in plane.chunks_exact(4) {
            assert_eq!(px, [1.0, 1.0, 1.0, 1.0]);
        }
        // The exporter is exactly this door + the §6.4 quantizer.
        assert_eq!(
            export_png_bytes(&e).unwrap(),
            encode_composite_png(w, h, &plane).unwrap(),
            "byte-stable by construction"
        );
    }

    #[test]
    fn import_decodes_srgb_to_linear_premultiplied_f32() {
        // sRGB 188 ≈ linear 0.5; alpha 128 → premultiply in linear.
        let png = tiny_png(&[[188, 0, 255, 255], [255, 255, 255, 0]], 2, 1);
        let (buf, w, h) = decode_png_to_buffer(&png, ProfileId::nil()).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(buf.kind(), ChannelKind::F32);
        assert_eq!(buf.model(), ColorModelKind::Rgba);
        let px = buf.read_pixel_f32(IVec2::new(0, 0));
        assert!((px[0] - srgb_decode(188.0 / 255.0)).abs() < 1e-6);
        assert_eq!(px[1], 0.0);
        assert!((px[2] - 1.0).abs() < 1e-6);
        assert_eq!(px[3], 1.0);
        // a = 0: premultiplied storage keeps no hidden color (§6.4).
        assert_eq!(buf.read_pixel_f32(IVec2::new(1, 0)), [0.0; 4]);
        assert!(buf.validate_premultiplied().is_ok());
        assert_eq!(
            buf.bounds(),
            IRect::new(0, 0, 1, 1),
            "pixel-exact bounds exclude a=0"
        );
    }

    #[test]
    fn import_round_trips_opaque_srgb_bytes_exactly() {
        // u8 sRGB → linear f32 → back to u8 sRGB is exact for opaque pixels.
        let samples: Vec<[u8; 4]> = (0..=255)
            .step_by(5)
            .map(|v| [v as u8, 255 - v as u8, 7, 255])
            .collect();
        let png = tiny_png(&samples, samples.len() as u32, 1);
        let (buf, _, _) = decode_png_to_buffer(&png, ProfileId::nil()).unwrap();
        for (i, s) in samples.iter().enumerate() {
            let px = buf.read_pixel_f32(IVec2::new(i as i32, 0));
            let back = [
                (srgb_encode(px[0]) * 255.0).round() as u8,
                (srgb_encode(px[1]) * 255.0).round() as u8,
                (srgb_encode(px[2]) * 255.0).round() as u8,
                (px[3] * 255.0).round() as u8,
            ];
            assert_eq!(&back, s);
        }
    }

    #[test]
    fn malformed_png_is_a_typed_read_error() {
        let err = decode_png_to_buffer(b"not a png at all", ProfileId::nil()).unwrap_err();
        assert_eq!(err.code, crate::engine::error::ErrorCode::IoReadFailed);
    }

    /// QA-F8: the reference export SERVES smart-object proxies (byte-stable
    /// across `smartobject.convert`) and REFUSES TYPED — never omits — when
    /// the composite cannot be represented. Since the §12.1 slice the
    /// enabled-chain refusal is the UNREGISTERED-KERNEL arm: this engine is
    /// a bare-core assembly (no OpImpls), so the enabled op cannot evaluate
    /// — same typed code, honest new reason. Disabled chain entries
    /// contribute identity (§12.4) without touching the registry at all.
    #[test]
    fn export_serves_smart_proxy_and_refuses_typed_on_enabled_chain() {
        use crate::engine::kind_impl::EngineAssembly;
        use serde_json::json;

        let mut e = crate::engine::kinds::register(EngineAssembly::new()).build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0e0f0000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 64, "height": 64, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];
        let pre = export_png_bytes(&e).unwrap();
        assert!(!export_is_smart_served(&e), "raster-only doc: legacy arm");

        let nl = uuid::Uuid::new_v4();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0e0f0000-0000-4000-8000-000000000002",
                "kind": "smartobject.convert",
                "params": {"layerIds": [bg.as_uuid().to_string()], "newLayerId": nl.to_string()},
            })
            .to_string(),
        )
        .unwrap();
        assert!(
            export_is_smart_served(&e),
            "smart layer is effectively visible"
        );
        let post = export_png_bytes(&e).unwrap();
        assert_eq!(
            pre, post,
            "convert preserves the look: the proxy serves the captured composite byte-for-byte"
        );

        // An ENABLED chain entry: the proxy no longer represents the
        // composite — typed refusal, never a silent serve (interim
        // discipline). Disabled again = identity ⇒ serves again.
        let so = crate::graph::id::LayerId::from_uuid(nl);
        let oid = crate::graph::id::OpId::from_uuid(uuid::Uuid::new_v4());
        {
            let d = e.doc_mut_for_test();
            d.insert_op(crate::graph::op_node::OpNode::new(
                oid,
                "adjustment.invert",
                json!({}),
            ))
            .unwrap();
            d.insert_smart_filter_ref(so, oid, 0).unwrap();
        }
        let err = export_png_bytes(&e).unwrap_err();
        assert_eq!(err.code, crate::engine::error::ErrorCode::NotImplemented);
        {
            let d = e.doc_mut_for_test();
            let op = d.op_mut(oid).unwrap();
            let mut block = op.value_block();
            block.enabled = false;
            op.apply_value_block(&block);
        }
        assert_eq!(
            export_png_bytes(&e).unwrap(),
            pre,
            "a disabled chain contributes identity (§12.4) — the proxy serves"
        );
    }

    fn export_is_smart_served(e: &crate::engine::Engine) -> bool {
        has_visible_parametric_content(e.document().unwrap())
    }

    /// An assembly with core kinds + the in-crate probe kernels — the
    /// §12.1 seam end-to-end without the tools crates (production kernels
    /// ride the same wraps; the CLI battery covers those).
    fn probe_assembly() -> crate::engine::kind_impl::EngineAssembly {
        use crate::engine::op_eval::probes::{ScaleOp, SpreadOp};
        crate::engine::kinds::register(crate::engine::kind_impl::EngineAssembly::new())
            .with_op(std::sync::Arc::new(ScaleOp))
            .with_op(std::sync::Arc::new(SpreadOp))
    }

    fn apply(e: &mut crate::engine::Engine, json: &serde_json::Value) {
        e.apply_json(&json.to_string())
            .unwrap_or_else(|err| panic!("{json} -> {:?} {}", err.code, err.message));
    }

    /// §12.1 PARAMETRIC RE-RENDER at the export door, adjustment arm: an
    /// enabled adjustment (probe scale op through the ASSEMBLY registry)
    /// affects the composite below; `op.toggle` off restores the
    /// pre-adjustment bytes exactly (§12.4 at the door); two runs over
    /// the same document are byte-identical, including across a real
    /// package save/load (determinism REQUIRED by the slice contract).
    #[test]
    fn export_evaluates_adjustment_over_composite_below_and_is_deterministic() {
        use serde_json::json;
        let mut e = probe_assembly().build();
        apply(
            &mut e,
            &json!({
                "schema": "appname.command.v1",
                "id": "0e120000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 8, "height": 8, "background": "white"},
            }),
        );
        let pre = export_png_bytes(&e).unwrap();
        apply(
            &mut e,
            &json!({
                "schema": "appname.command.v1",
                "id": "0e120000-0000-4000-8000-000000000002",
                "kind": "adjustment.add",
                "params": {
                    "opKind": "x.test.scale",
                    "newLayerId": "0e120000-0000-4000-8000-00000000aaa1",
                    "newOpId": "0e120000-0000-4000-8000-00000000aaa2",
                    "params": {"factor": 0.5},
                },
            }),
        );
        let served = export_png_bytes(&e).unwrap();
        assert_ne!(served, pre, "an enabled adjustment changes the composite");
        // Pixel truth: white backdrop scaled by 0.5 in linear light →
        // sRGB-encoded 0.5 in the 8-bit export.
        let (buf, _, _) = decode_png_to_buffer(&served, ProfileId::nil()).unwrap();
        let px = buf.read_pixel_f32(IVec2::new(4, 4));
        let want = srgb_decode((srgb_encode(0.5) * 255.0).round() / 255.0);
        assert!(
            (px[0] - want).abs() < 1e-6 && (px[3] - 1.0).abs() < 1e-6,
            "adjustment applied to the backdrop: {px:?} vs {want}"
        );
        // Two-run byte-identity on the SAME engine…
        assert_eq!(served, export_png_bytes(&e).unwrap(), "pure re-render");
        // …and across a real package round trip through the SAME assembly.
        let pkg = crate::io::package::save_package_bytes(&e).unwrap();
        let loaded = crate::io::package::load_package_bytes_with(&probe_assembly(), &pkg).unwrap();
        assert_eq!(
            export_png_bytes(&loaded.engine).unwrap(),
            served,
            "reopened parametric doc re-renders byte-identically"
        );
        // §12.4 at the door: toggling the op off == never applied, bytes.
        apply(
            &mut e,
            &json!({
                "schema": "appname.command.v1",
                "id": "0e120000-0000-4000-8000-000000000003",
                "kind": "op.toggle",
                "params": {"opId": "0e120000-0000-4000-8000-00000000aaa2", "enabled": false},
            }),
        );
        assert_eq!(
            export_png_bytes(&e).unwrap(),
            pre,
            "disabled adjustment exports the pre-add bytes exactly (§12.4)"
        );
    }

    /// §12.1 chain arm: an enabled smart-filter chain evaluates OVER the
    /// value-captured proxy at the export door (the proxy itself stays
    /// the captured SOURCE); toggling the entry off re-serves the plain
    /// proxy byte-for-byte; a convolution probe (roi ≠ identity) serves
    /// deterministically through the apron path.
    #[test]
    fn export_evaluates_enabled_smart_chain_over_the_proxy() {
        use serde_json::json;
        let mut e = probe_assembly().build();
        apply(
            &mut e,
            &json!({
                "schema": "appname.command.v1",
                "id": "0e130000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 8, "height": 8, "background": "white"},
            }),
        );
        let bg = e.document().unwrap().z_order_walk()[0];
        apply(
            &mut e,
            &json!({
                "schema": "appname.command.v1",
                "id": "0e130000-0000-4000-8000-000000000002",
                "kind": "smartobject.convert",
                "params": {
                    "layerIds": [bg.as_uuid().to_string()],
                    "newLayerId": "0e130000-0000-4000-8000-00000000bbb1",
                },
            }),
        );
        let proxy_only = export_png_bytes(&e).unwrap();
        let so = crate::graph::id::LayerId::from_uuid(
            uuid::Uuid::parse_str("0e130000-0000-4000-8000-00000000bbb1").unwrap(),
        );
        let oid = crate::graph::id::OpId::from_uuid(
            uuid::Uuid::parse_str("0e130000-0000-4000-8000-00000000ccc1").unwrap(),
        );
        {
            let d = e.doc_mut_for_test();
            d.insert_op(crate::graph::op_node::OpNode::new(
                oid,
                "x.test.scale",
                json!({"factor": 0.25}),
            ))
            .unwrap();
            d.insert_smart_filter_ref(so, oid, 0).unwrap();
        }
        let served = export_png_bytes(&e).unwrap();
        assert_ne!(served, proxy_only, "the enabled chain changes the export");
        let (buf, _, _) = decode_png_to_buffer(&served, ProfileId::nil()).unwrap();
        let px = buf.read_pixel_f32(IVec2::new(3, 3));
        let want = srgb_decode((srgb_encode(0.25) * 255.0).round() / 255.0);
        assert!(
            (px[0] - want).abs() < 1e-6,
            "chain evaluated over the proxy: {px:?} vs {want}"
        );
        assert_eq!(served, export_png_bytes(&e).unwrap(), "two-run identity");
        // Toggle the entry off: the proxy serves plain again, byte-true.
        {
            let d = e.doc_mut_for_test();
            let op = d.op_mut(oid).unwrap();
            let mut b = op.value_block();
            b.enabled = false;
            op.apply_value_block(&b);
        }
        assert_eq!(
            export_png_bytes(&e).unwrap(),
            proxy_only,
            "disabled chain entry re-serves the captured proxy (§12.4)"
        );
        // A convolution probe (roi ≠ identity): the uniform interior stays
        // uniform; the canvas-edge pixel darkens (taps beyond the proxy
        // content read transparent) — and the serve is deterministic.
        {
            let d = e.doc_mut_for_test();
            let op = d.op_mut(oid).unwrap();
            let mut b = op.value_block();
            b.enabled = true;
            op.apply_value_block(&b);
            let oid2 = crate::graph::id::OpId::from_uuid(
                uuid::Uuid::parse_str("0e130000-0000-4000-8000-00000000ccc2").unwrap(),
            );
            d.insert_op(crate::graph::op_node::OpNode::new(
                oid2,
                "x.test.spread",
                json!({}),
            ))
            .unwrap();
            d.insert_smart_filter_ref(so, oid2, 1).unwrap();
        }
        let convolved = export_png_bytes(&e).unwrap();
        assert_eq!(convolved, export_png_bytes(&e).unwrap(), "deterministic");
        let (buf, _, _) = decode_png_to_buffer(&convolved, ProfileId::nil()).unwrap();
        let center = buf.read_pixel_f32(IVec2::new(4, 4));
        let edge = buf.read_pixel_f32(IVec2::new(0, 4));
        assert!(
            edge[3] < center[3],
            "edge coverage drops through the 3-tap spread: {edge:?} vs {center:?}"
        );
    }

    /// Reviewer MINOR (CP-0.7 close), §12.1-slice posture: a crafted
    /// PACKAGE carrying a visible ENABLED adjustment layer routes to the
    /// parametric arm; in a BARE-core assembly (no OpImpls registered) its
    /// kernel cannot evaluate, so export refuses TYPED — the legacy
    /// `pixels()` walk would silently omit it (the banned shape). In
    /// assembled engines the same document SERVES (the CLI battery); the
    /// refusal here survives a real save/load round trip.
    #[test]
    fn crafted_package_with_visible_adjustment_layer_refuses_typed_export() {
        use crate::engine::kind_impl::EngineAssembly;
        use serde_json::json;

        let mut e = crate::engine::kinds::register(EngineAssembly::new()).build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0e0f0000-0000-4000-8000-000000000011",
                "kind": "doc.new",
                "params": {"width": 32, "height": 32, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        assert!(export_png_bytes(&e).is_ok(), "raster-only doc exports");

        let oid = crate::graph::id::OpId::from_uuid(uuid::Uuid::new_v4());
        let lid = crate::graph::id::LayerId::from_uuid(uuid::Uuid::new_v4());
        {
            let d = e.doc_mut_for_test();
            let root = d.root();
            d.insert_op(crate::graph::op_node::OpNode::new(
                oid,
                "adjustment.curves",
                json!({}),
            ))
            .unwrap();
            d.insert_child(
                root,
                1,
                crate::graph::node::LayerNode::new_adjustment(lid, "Curves", oid),
            )
            .unwrap();
        }
        assert!(
            has_visible_parametric_content(e.document().unwrap()),
            "visible adjustment layer widens the routing predicate (reviewer MINOR)"
        );
        let err = export_png_bytes(&e).unwrap_err();
        assert_eq!(
            err.code,
            crate::engine::error::ErrorCode::NotImplemented,
            "typed refusal, never a silent omit: {}",
            err.message
        );

        // The crafted-package leg proper: save, reload through the real
        // loader, and the refusal survives the round trip.
        let bytes = crate::io::package::save_package_bytes(&e).unwrap();
        let loaded = crate::io::package::load_package_bytes(&bytes).unwrap();
        let err = export_png_bytes(&loaded.engine).unwrap_err();
        assert_eq!(
            err.code,
            crate::engine::error::ErrorCode::NotImplemented,
            "loader-constructed adjustment layer refuses typed too: {}",
            err.message
        );
    }
}
