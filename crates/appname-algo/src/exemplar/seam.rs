// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §45.3 constant table "Seam pass" row (per level: Kwatra
//! overlap energy, 4-connected, α-expansion to convergence ≤ 4 sweeps)
//! and "Voting/ownership" row (hard patch ownership + seam relabel — no
//! soft multi-patch voting); §21.4 COMMIT stage ("one GRAPH-CUT SEAM pass
//! over ownership boundaries … relabeling pixels to the lower-energy
//! owner"). Clean-room from kwatra03 (seam energy) with the expansion
//! move of the expired α-expansion machinery (register P-6744923,
//! "Expired - Lifetime"); the min-cut kernel is the wave-2
//! [`crate::graphcut`] engine (reuse, not rebuilt).
//!
//! Energy: for a 4-connected pixel pair (p, q) with owners A ≠ B, the
//! seam cost is the kwatra03 overlap term
//! `M(p,q,A,B) = ‖A(p) − B(p)‖ + ‖A(q) − B(q)‖` (L2 over channels),
//! where a label's color at a pixel is its library patch sample when the
//! pixel lies in that patch's footprint and the current buffer value
//! otherwise (documented closure of the energy outside footprints).
//!
//! Each expansion move is a small binary min-cut confined to one patch's
//! 7×7 footprint (the only region where that label is defined), built
//! with the standard submodular pairwise decomposition (the energy is a
//! metric: M(α,α) = 0 and triangle inequality holds for norms, so
//! `E(0,1)+E(1,0) ≥ E(0,0)` always). Sweeps visit patches in ascending
//! id order — fully deterministic; the pass never touches search state
//! (rights invariants live in [`crate::exemplar::synth`]; this pass only
//! relabels committed ownership).

use crate::exemplar::descriptor::PATCH_RADIUS;
use crate::exemplar::synth::{LevelLibs, LevelSynth, SrcRef, NO_OWNER};
use crate::graphcut::CutGraph;
use crate::plane::Plane;

const FOUR: [(isize, isize); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];

/// A label's color at a pixel (see module docs), per channel into `out`.
#[allow(clippy::too_many_arguments)]
fn label_color(
    label: u32,
    x: usize,
    y: usize,
    planes: &[Plane],
    libs: &LevelLibs,
    state: &LevelSynth,
    w: usize,
    out: &mut [f64],
) {
    if label != NO_OWNER {
        let pr = &state.patches[label as usize];
        let dx = x as isize - pr.tx as isize;
        let dy = y as isize - pr.ty as isize;
        let r = PATCH_RADIUS as isize;
        if dx.abs() <= r && dy.abs() <= r {
            let lx = (pr.sx as isize + dx) as usize;
            let ly = (pr.sy as isize + dy) as usize;
            let lib = &libs.libs[pr.lib as usize];
            for (c, plane) in lib.planes.iter().enumerate() {
                out[c] = plane.data[ly * w + lx] as f64;
            }
            return;
        }
    }
    for (c, plane) in planes.iter().enumerate() {
        out[c] = plane.data[y * w + x] as f64;
    }
}

/// Kwatra pair cost `‖A(p)−B(p)‖ + ‖A(q)−B(q)‖` for labels `(a, b)` at
/// the 4-connected pair `(p, q)`.
#[allow(clippy::too_many_arguments)]
fn pair_cost(
    a: u32,
    b: u32,
    p: (usize, usize),
    q: (usize, usize),
    planes: &[Plane],
    libs: &LevelLibs,
    state: &LevelSynth,
    w: usize,
    scratch: &mut [f64],
) -> f64 {
    let ch = planes.len();
    let (ca, cb) = scratch.split_at_mut(ch);
    let mut total = 0f64;
    for &(x, y) in &[p, q] {
        label_color(a, x, y, planes, libs, state, w, ca);
        label_color(b, x, y, planes, libs, state, w, cb);
        let mut acc = 0f64;
        for c in 0..ch {
            let d = ca[c] - cb[c];
            acc += d * d;
        }
        total += acc.sqrt();
    }
    total
}

/// One §45.3 seam pass: α-expansion sweeps (ascending patch id) until no
/// relabel occurs, capped at 4 sweeps. Relabeling copies the winning
/// owner's pixels into the buffer and updates the recorded sources.
pub(crate) fn seam_pass(
    planes: &mut [Plane],
    hole: &[bool],
    w: usize,
    h: usize,
    libs: &LevelLibs,
    state: &mut LevelSynth,
) {
    let r = PATCH_RADIUS as isize;
    let ch = planes.len();
    let mut scratch = vec![0f64; ch * 2];
    for _sweep in 0..4 {
        let mut changed = false;
        for alpha in 0..state.patches.len() as u32 {
            let pr = state.patches[alpha as usize];
            // Footprint variables: hole pixels owned by a DIFFERENT patch.
            let mut nodes: Vec<(usize, usize)> = Vec::new();
            let mut node_of = vec![usize::MAX; (2 * r as usize + 1) * (2 * r as usize + 1)];
            let mut fi = 0usize;
            for dy in -r..=r {
                for dx in -r..=r {
                    let (xx, yy) = (pr.tx as isize + dx, pr.ty as isize + dy);
                    let slot = fi;
                    fi += 1;
                    if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                        continue;
                    }
                    let (x, y) = (xx as usize, yy as usize);
                    let i = y * w + x;
                    if hole[i] && state.owner[i] != alpha && state.owner[i] != NO_OWNER {
                        node_of[slot] = nodes.len();
                        nodes.push((x, y));
                    }
                }
            }
            if nodes.is_empty() {
                continue;
            }
            let node_at = |x: usize, y: usize| -> Option<usize> {
                let dx = x as isize - pr.tx as isize;
                let dy = y as isize - pr.ty as isize;
                if dx.abs() > r || dy.abs() > r {
                    return None;
                }
                let slot = ((dy + r) * (2 * r + 1) + (dx + r)) as usize;
                let n = node_of[slot];
                if n == usize::MAX {
                    None
                } else {
                    Some(n)
                }
            };
            // Signed unary accumulators: cost(x=0 keep) / cost(x=1 α).
            let mut u0 = vec![0f64; nodes.len()];
            let mut u1 = vec![0f64; nodes.len()];
            let mut edges: Vec<(usize, usize, f64)> = Vec::new();
            for (pi, &(px, py)) in nodes.iter().enumerate() {
                let lp = state.owner[py * w + px];
                for (dx, dy) in FOUR {
                    let (qx, qy) = (px as isize + dx, py as isize + dy);
                    if qx < 0 || qy < 0 || qx >= w as isize || qy >= h as isize {
                        continue;
                    }
                    let (qx, qy) = (qx as usize, qy as usize);
                    match node_at(qx, qy) {
                        Some(qi) => {
                            // Count each variable pair once.
                            if qi <= pi {
                                continue;
                            }
                            let lq = state.owner[qy * w + qx];
                            let a = pair_cost(
                                lp,
                                lq,
                                (px, py),
                                (qx, qy),
                                planes,
                                libs,
                                state,
                                w,
                                &mut scratch,
                            );
                            let b = pair_cost(
                                lp,
                                alpha,
                                (px, py),
                                (qx, qy),
                                planes,
                                libs,
                                state,
                                w,
                                &mut scratch,
                            );
                            let cc = pair_cost(
                                alpha,
                                lq,
                                (px, py),
                                (qx, qy),
                                planes,
                                libs,
                                state,
                                w,
                                &mut scratch,
                            );
                            // E(0,0)=a, E(0,1)=b, E(1,0)=cc, E(1,1)=0.
                            u0[pi] += b;
                            u0[qi] += a - b;
                            let m = (b + cc - a).max(0.0);
                            if m > 0.0 {
                                edges.push((pi, qi, m));
                            }
                        }
                        None => {
                            // Fixed neighbor: its label is α where owned
                            // by α, otherwise its own owner/buffer.
                            let lq = state.owner[qy * w + qx];
                            u0[pi] += pair_cost(
                                lp,
                                lq,
                                (px, py),
                                (qx, qy),
                                planes,
                                libs,
                                state,
                                w,
                                &mut scratch,
                            );
                            u1[pi] += pair_cost(
                                alpha,
                                lq,
                                (px, py),
                                (qx, qy),
                                planes,
                                libs,
                                state,
                                w,
                                &mut scratch,
                            );
                        }
                    }
                }
            }
            let mut g = CutGraph::with_capacity(nodes.len(), nodes.len() + edges.len());
            for i in 0..nodes.len() {
                let mn = u0[i].min(u1[i]);
                g.add_terminal(i, u0[i] - mn, u1[i] - mn);
            }
            for &(p, q, m) in &edges {
                // Paid iff x_p = 1 (source side) and x_q = 0 (sink side).
                g.add_edge(p, q, m, 0.0);
            }
            g.solve();
            let take = g.source_side();
            let lib = &libs.libs[pr.lib as usize];
            for (ni, &(x, y)) in nodes.iter().enumerate() {
                if !take[ni] {
                    continue;
                }
                let i = y * w + x;
                let dx = x as isize - pr.tx as isize;
                let dy = y as isize - pr.ty as isize;
                let lx = (pr.sx as isize + dx) as usize;
                let ly = (pr.sy as isize + dy) as usize;
                for (c, plane) in lib.planes.iter().enumerate() {
                    planes[c].data[i] = plane.data[ly * w + lx];
                }
                state.owner[i] = alpha;
                state.src[i] = SrcRef {
                    lib: pr.lib,
                    x: lx as u16,
                    y: ly as u16,
                };
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}
