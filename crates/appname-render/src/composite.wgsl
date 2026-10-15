// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// appname-render composite shader — the committed GPU contract for the
// §4.5 production dispatcher (`cs_composite_full`).
//
// Spec: ES §4.4 (Porter-Duff wrapper), §4.5 (27-mode catalog; Dissolve
// NORMATIVE; production dispatcher routing), Appendix A (the NORMATIVE
// formula table — VividLight is the burn/dodge composition per the
// table, NOT the §4.5 inline sample's self-described "simplified"
// case 15u; O-RENDER lane-log observation), §34.3 (parity ≤ 1/255 vs
// the §4.8 CPU oracle; Dissolve bit-exact vs its own baseline).
//
// CPU-pinned semantics this shader mirrors (slice-A pins, lane log):
//  * un-premultiply: EXACT guarded divide — `select(rgb / a, 0, a == 0)`
//    matching the §4.8 CPU reference, NOT the §4.5 sample's
//    `max(a, 1e-6)` clamp (decision recorded with the parity harness;
//    divergence domain of the clamp form is α < 1e-6 × HDR).
//  * ClipColor HDR-degenerate strict-inequality guards → the formula's
//    continuous-limit values (1-triple / 0-triple) — never NaN
//    (`hdr_degenerate_clip_color_pins_limit_values`).
//  * whole-pixel ties → backdrop (lum <= / >= select orientation).
//  * HardMix: (b + s) >= 1.0 → 1 else 0 (§4.5 text + WGSL form;
//    Appendix-A equivalence note in the lane log for the fold-in).
//  * Dissolve: n = (BN64[(p_doc + off) & 63] + 0.5) / 256, harden
//    aS = select(0, 1, a_eff > n), then the FULL Porter-Duff expression
//    with B = Cs (no shortcut) — matches the CPU kernel bit-for-bit in
//    structure; cross-path bit-equality is the §34.3 expectation.
//  * zero-alpha UNIFORM sources never reach this kernel — the host
//    compiles the normative skip (CP-0.3 O-INT ruling), not the shader.
//
// Target model: PING-PONG, not read-write — `dst_tex` is the sampled
// backdrop input and `out_tex` a write-only rgba32float storage target
// (core WebGPU permits read_write storage only for r32 formats; the
// §4.5 sample's read_write dst is illustrative, and §4.1's pooled
// transient targets anticipate ping-pong). The host swaps roles
// between passes of one tile's composite chain.

struct Uniforms {
    mode: u32,
    opacity: f32,
    bn_off: vec2<i32>,
    tile_origin: vec2<i32>,
}

@group(0) @binding(0) var src_tex: texture_2d<f32>;    // premult linear source
@group(0) @binding(1) var dst_tex: texture_2d<f32>;    // premult linear backdrop (in)
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba32float, write>;
@group(0) @binding(3) var bn_tex: texture_2d<u32>;     // r8uint 64x64 BN64
@group(0) @binding(4) var<uniform> u: Uniforms;
// §11.2 effective coverage E, per pixel, r32float — host-computed by the
// ONE shared routine (appname_core::resolve_effective_coverage) and
// uploaded verbatim; 1.0 = unmasked (an exact multiplicative identity, so
// unmasked composites stay bit-for-bit the un-masked kernel).
@group(0) @binding(5) var cov_tex: texture_2d<f32>;

const NONSEP_BASE: u32 = 23u;

fn hard_light(b: f32, s: f32) -> f32 {
    return select(1.0 - 2.0 * (1.0 - b) * (1.0 - s), 2.0 * b * s, s <= 0.5);
}
fn soft_light(b: f32, s: f32) -> f32 {
    let d = select(sqrt(b), ((16.0 * b - 12.0) * b + 4.0) * b, b <= 0.25);
    return select(b + (2.0 * s - 1.0) * (d - b), b - (1.0 - 2.0 * s) * b * (1.0 - b), s <= 0.5);
}
// W3C Compositing and Blending Level 1 three-case guard, bit-identical to
// the CPU oracle (blend.rs): the leading backdrop case (b==1 burn / b==0
// dodge) precedes the source case. select() evaluates both arms, but the
// outer select pins the backdrop endpoint regardless of the inner value.
fn color_burn(b: f32, s: f32) -> f32 {
    return select(select(1.0 - min(1.0, (1.0 - b) / s), 0.0, s == 0.0), 1.0, b == 1.0);
}
fn color_dodge(b: f32, s: f32) -> f32 {
    return select(select(min(1.0, b / (1.0 - s)), 1.0, s == 1.0), 0.0, b == 0.0);
}
// Appendix A row 15 (normative): the burn/dodge composition.
fn vivid_light(b: f32, s: f32) -> f32 {
    return select(color_dodge(b, 2.0 * s - 1.0), color_burn(b, 2.0 * s), s <= 0.5);
}

fn blend_sep(mode: u32, b: f32, s: f32) -> f32 {
    switch mode {
        case 0u:  { return s; }                                        // Normal
        case 2u:  { return min(b, s); }                                // Darken
        case 3u:  { return b * s; }                                    // Multiply
        case 4u:  { return color_burn(b, s); }                         // Color Burn
        case 5u:  { return max(0.0, b + s - 1.0); }                    // Linear Burn
        case 7u:  { return max(b, s); }                                // Lighten
        case 8u:  { return b + s - b * s; }                            // Screen
        case 9u:  { return color_dodge(b, s); }                        // Color Dodge
        case 10u: { return min(1.0, b + s); }                          // Linear Dodge (Add)
        case 12u: { return hard_light(s, b); }                         // Overlay
        case 13u: { return soft_light(b, s); }                         // Soft Light
        case 14u: { return hard_light(b, s); }                         // Hard Light
        case 15u: { return vivid_light(b, s); }                        // Vivid Light (Appendix A)
        case 16u: { return clamp(b + 2.0 * s - 1.0, 0.0, 1.0); }       // Linear Light
        case 17u: { return select(max(b, 2.0 * s - 1.0), min(b, 2.0 * s), s <= 0.5); } // Pin Light
        case 18u: { return select(0.0, 1.0, (b + s) >= 1.0); }         // Hard Mix
        case 19u: { return abs(b - s); }                               // Difference
        case 20u: { return b + s - 2.0 * b * s; }                      // Exclusion
        case 21u: { return max(0.0, b - s); }                          // Subtract
        case 22u: { return select(min(1.0, b / s), 1.0, s == 0.0); }   // Divide
        default:  { return s; }
    }
}

// Lum in the EXPLICIT CPU evaluation order — (0.3·r + 0.59·g) + 0.11·b
// — not dot(): WGSL's dot() accuracy is implementation-inherited, and
// the whole-pixel modes BRANCH on lum equality at full amplitude, so
// branch-input bit-equality must ride only correctly-rounded ops
// (slice-D review MINOR-3 hard fix; mul/add are correctly rounded by
// the WGSL accuracy table).
fn lum(c: vec3<f32>) -> f32 { return 0.3 * c.r + 0.59 * c.g + 0.11 * c.b; }
fn sat(c: vec3<f32>) -> f32 {
    return max(max(c.r, c.g), c.b) - min(min(c.r, c.g), c.b);
}
// ClipColor with the slice-A HDR-degenerate strict-inequality guards
// (header note): uniform out-of-range triples take the continuous-limit
// values instead of 0·inf = NaN. Mirrors the CPU pin exactly.
fn clip_color(c_in: vec3<f32>) -> vec3<f32> {
    var c = c_in;
    let l = lum(c);
    let n = min(min(c.r, c.g), c.b);
    let x = max(max(c.r, c.g), c.b);
    if (n < 0.0) {
        if (l > n) { c = vec3(l) + (c - vec3(l)) * l / (l - n); }
        else { c = vec3(0.0); }
    }
    if (x > 1.0) {
        if (x > l) { c = vec3(l) + (c - vec3(l)) * (1.0 - l) / (x - l); }
        else { c = vec3(1.0); }
    }
    return c;
}
fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
    return clip_color(c + vec3(l - lum(c)));
}
fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    let cmin = min(min(c.r, c.g), c.b);
    let cmax = max(max(c.r, c.g), c.b);
    if (cmax > cmin) { return (c - vec3(cmin)) * s / (cmax - cmin); }
    return vec3<f32>(0.0);
}
fn blend_nonsep(mode: u32, cb: vec3<f32>, cs: vec3<f32>) -> vec3<f32> {
    switch mode {
        case 23u: { return set_lum(set_sat(cs, sat(cb)), lum(cb)); }  // Hue
        case 24u: { return set_lum(set_sat(cb, sat(cs)), lum(cb)); }  // Saturation
        case 25u: { return set_lum(cs, lum(cb)); }                    // Color
        case 26u: { return set_lum(cb, lum(cs)); }                    // Luminosity
        default:  { return cs; }
    }
}
// Whole-pixel picks; ties → backdrop [NORMATIVE §4.5].
fn blend_darker_color(cb: vec3<f32>, cs: vec3<f32>) -> vec3<f32> {
    return select(cs, cb, lum(cb) <= lum(cs));
}
fn blend_lighter_color(cb: vec3<f32>, cs: vec3<f32>) -> vec3<f32> {
    return select(cs, cb, lum(cb) >= lum(cs));
}

// Correctly rounded p/a for the un-premultiply (usage note at the call
// site): one Newton refinement of the reciprocal, then the Markstein
// fma-residual correction. GIVEN a fused (single-rounded) fma this
// yields the round-to-nearest quotient — bit-equal to the CPU's IEEE
// division. WGSL does NOT contract fma fusion (its accuracy table
// inherits from a*b+c; MSL pins the fused op, other backends may not),
// so cross-path bit-equality here is a PER-BACKEND EXPECTATION
// validated empirically by the §34.3 gate on each runner tier — green
// on Metal — not a WGSL-level guarantee (slice-D review MAJOR-2).
// Domain hazard, real and recorded (lane log, slice-D closures entry):
// a SUBNORMAL a (legal-at-rest f32 tile data; §6.4 bans non-finite
// values only) overflows the reciprocal seed to +inf and this sequence
// returns NaN/FTZ-dependent values where the CPU returns a finite
// quotient. Outside the corpus domain and unreachable from canonical
// u8/u16 content; a working-domain alpha floor is a fold-in question,
// not a renderer election.
fn div_cr(p: vec3<f32>, a: f32) -> vec3<f32> {
    var y = 1.0 / a;                       // seed, ≤ 2.5 ULP
    y = fma(fma(-a, y, 1.0), y, y);        // Newton: error → O(e²)
    let q = p * vec3(y);                   // candidate quotient
    let r = fma(vec3(-a), q, p);           // exact residual (single-rounded fma)
    return fma(r, vec3(y), q);             // Markstein correction
}

// The §4.5 production dispatcher: complete routing, one dispatch per
// tile, 16×16 workgroups over 256×256.
@compute @workgroup_size(16, 16)
fn cs_composite_full(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec2<i32>(gid.xy);
    if (p.x >= 256 || p.y >= 256) { return; }
    // §11.2 mask the premultiplied source: multiply ALL 4 channels by the
    // host-computed coverage E (`src *= coverage`). A SINGLE correctly-
    // rounded multiply by the IDENTICAL E the CPU applies (apply_coverage),
    // so cross-path parity here is exact — as strong as the un-masked
    // kernel (E == 1.0 leaves src bit-unchanged). gpu.rs determinism note.
    let sb = textureLoad(src_tex, p, 0) * textureLoad(cov_tex, p, 0).r;
    let bb = textureLoad(dst_tex, p, 0);
    var aS = sb.a * u.opacity;
    let aB = bb.a;
    // Exact guarded un-premultiply — CORRECTLY ROUNDED, bit-equal to
    // the CPU oracle's IEEE division (div_cr below), with the x/x == 1
    // identity additionally PINNED by select. Why bit-equality here is
    // load-bearing (Metal parity-run findings, lane log): WGSL native
    // '/' carries up to 2.5 ULP, and (1) an exact-equality formula
    // guard (ColorDodge s == 1) can be slipped into its pole — measured
    // 1e9 blowup; (2) DISCONTINUOUS formulas (HardMix's b+s >= 1 step;
    // the whole-pixel Lum picks) branch on these values, so a 1-ULP
    // input wobble flips a step of height 1 — measured 60/255. Making
    // the un-premultiply bit-exact makes every downstream branch
    // decision identical cross-path; remaining in-formula divisions
    // feed only continuous expressions (guarded poles).
    let Cs = select(
        select(div_cr(sb.rgb, sb.a), vec3(1.0), sb.rgb == vec3(sb.a)),
        vec3(0.0),
        sb.a == 0.0,
    );
    let Cb = select(
        select(div_cr(bb.rgb, bb.a), vec3(1.0), bb.rgb == vec3(bb.a)),
        vec3(0.0),
        bb.a == 0.0,
    );
    var Bc: vec3<f32>;
    if (u.mode == 1u) {
        // Dissolve [NORMATIVE §4.5]: harden aS against BN64 in document
        // space; B is Normal. Full Porter-Duff expression, no shortcut.
        let q = (u.tile_origin + p + u.bn_off) & vec2<i32>(63, 63);
        // ×(1/256) not /256: multiplication is correctly rounded by
        // the WGSL accuracy table while division carries 2.5 ULP; the
        // reciprocal is exact (2^-8), so the threshold is CONTRACTUALLY
        // bit-equal to the CPU's (v + 0.5)/256 — not just empirically
        // (slice-D review observation).
        let n = (f32(textureLoad(bn_tex, q, 0).r) + 0.5) * (1.0 / 256.0);
        aS = select(0.0, 1.0, aS > n);
        Bc = Cs;
    } else if (u.mode == 6u) {
        Bc = blend_darker_color(Cb, Cs);
    } else if (u.mode == 11u) {
        Bc = blend_lighter_color(Cb, Cs);
    } else if (u.mode >= NONSEP_BASE) {
        Bc = blend_nonsep(u.mode, Cb, Cs);
    } else {
        Bc = vec3(blend_sep(u.mode, Cb.r, Cs.r),
                  blend_sep(u.mode, Cb.g, Cs.g),
                  blend_sep(u.mode, Cb.b, Cs.b));
    }
    let Co = aS * (1.0 - aB) * Cs + aS * aB * Bc + (1.0 - aS) * aB * Cb;
    let aO = aS + aB * (1.0 - aS);
    textureStore(out_tex, p, vec4(Co, aO));
}
