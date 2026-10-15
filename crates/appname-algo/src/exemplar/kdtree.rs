// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §45.3 constant table "Index" row — k-d tree over the projected
//! S-patch descriptors, leaf ≤ 16, query = the top-8 nearest leaves
//! (`draft`: top-4) whose members are then re-ranked by exact 7×7 SSD by
//! the caller; §21.4 DESCRIPTORS stage (hertzmann01-lineage ANN patch
//! search, clean-room from the paper).
//!
//! STRUCTURAL RIGHTS INVARIANTS (contract `content_aware_fill` rows;
//! ES §45.3 [LEGAL]): this index is built ONCE per (level, library) as a
//! deterministic function of the source region alone, and it is IMMUTABLE
//! afterwards — [`KdTree::query_leaves`] takes `&self`, so the type system
//! itself rules out any per-query state, any per-pixel best-match store,
//! and any carry-over between queries. Every query's candidate set is
//! recomputed fresh from (tree, query point); nothing computed for one
//! front pixel seeds, biases, or warm-starts another pixel's search, and
//! no candidate window is ever sampled or perturbed — the traversal below
//! is exhaustive best-first over axis-aligned cell bounds with total
//! tie-breaking, a closed-form function of its arguments.
//!
//! Determinism: splits choose the widest dimension (ties: lowest index),
//! split at the sorted median (order: coordinate `total_cmp`, then point
//! index); best-first queries order cells by (bound bits, node id).

use std::collections::BinaryHeap;

/// Best-first traversal entry. Ordering is (smallest bound first, then
/// smallest node id) — total, because each tree node enters the heap at
/// most once; the offset payload never participates in ordering.
struct Visit {
    bound_bits: u64,
    node: u32,
    offs: Box<[f64]>,
}

impl PartialEq for Visit {
    fn eq(&self, other: &Self) -> bool {
        self.bound_bits == other.bound_bits && self.node == other.node
    }
}
impl Eq for Visit {}
impl Ord for Visit {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reversed: BinaryHeap is a max-heap, we want the min bound first.
        other
            .bound_bits
            .cmp(&self.bound_bits)
            .then(other.node.cmp(&self.node))
    }
}
impl PartialOrd for Visit {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

enum Node {
    Split {
        dim: u16,
        val: f64,
        left: u32,
        right: u32,
    },
    Leaf {
        start: u32,
        end: u32,
    },
}

/// Immutable k-d tree over `n` points in `k`-dim descriptor space.
pub struct KdTree {
    k: usize,
    /// Permutation: leaf slots → original point indices. (The point
    /// coordinates themselves are not retained: leaves hand their member
    /// indices to the caller's exact-SSD re-rank, §45.3.)
    order: Vec<u32>,
    nodes: Vec<Node>,
    root: u32,
}

/// Leaf capacity (§45.3: leaf ≤ 16).
pub const LEAF_CAP: usize = 16;

impl KdTree {
    /// Build from `n × k` row-major descriptor coordinates.
    pub fn build(coords: Vec<f64>, k: usize) -> KdTree {
        assert!(k > 0);
        let n = coords.len() / k;
        let mut order: Vec<u32> = (0..n as u32).collect();
        let mut nodes = Vec::new();
        let root = if n == 0 {
            nodes.push(Node::Leaf { start: 0, end: 0 });
            0
        } else {
            Self::build_rec(&coords, k, &mut order, 0, n, &mut nodes)
        };
        KdTree {
            k,
            order,
            nodes,
            root,
        }
    }

    fn build_rec(
        coords: &[f64],
        k: usize,
        order: &mut [u32],
        start: usize,
        end: usize,
        nodes: &mut Vec<Node>,
    ) -> u32 {
        let count = end - start;
        if count <= LEAF_CAP {
            nodes.push(Node::Leaf {
                start: start as u32,
                end: end as u32,
            });
            return (nodes.len() - 1) as u32;
        }
        // Widest dimension over the slice (ties: lowest dim index).
        let mut best_dim = 0usize;
        let mut best_spread = -1.0f64;
        for d in 0..k {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for &pi in &order[start..end] {
                let v = coords[pi as usize * k + d];
                lo = lo.min(v);
                hi = hi.max(v);
            }
            let spread = hi - lo;
            if spread > best_spread {
                best_spread = spread;
                best_dim = d;
            }
        }
        // Median split with total order (coordinate bits, then index).
        order[start..end].sort_by(|&a, &b| {
            coords[a as usize * k + best_dim]
                .total_cmp(&coords[b as usize * k + best_dim])
                .then(a.cmp(&b))
        });
        let mid = start + count / 2;
        let val = coords[order[mid] as usize * k + best_dim];
        let left = Self::build_rec(coords, k, order, start, mid, nodes);
        let right = Self::build_rec(coords, k, order, mid, end, nodes);
        nodes.push(Node::Split {
            dim: best_dim as u16,
            val,
            left,
            right,
        });
        (nodes.len() - 1) as u32
    }

    /// Number of points indexed.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// True when the tree indexes no points.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Collect the original point indices of the `top_l` leaves nearest to
    /// `q` (best-first by cell lower-bound distance; total tie-break by
    /// node id). The CALLER re-ranks these by exact 7×7 SSD (§45.3).
    ///
    /// Pure with respect to `self` (`&self` receiver): repeated identical
    /// queries produce identical candidate sets, recomputed fresh.
    pub fn query_leaves(&self, q: &[f64], top_l: usize, out: &mut Vec<u32>) {
        out.clear();
        debug_assert_eq!(q.len(), self.k);
        if self.order.is_empty() || top_l == 0 {
            return;
        }
        // Bounds are ≥ 0 so the f64 bit pattern orders monotonically.
        let mut heap: BinaryHeap<Visit> = BinaryHeap::new();
        heap.push(Visit {
            bound_bits: 0,
            node: self.root,
            offs: vec![0.0; self.k].into_boxed_slice(),
        });
        let mut leaves_taken = 0usize;
        while let Some(Visit {
            bound_bits,
            node,
            offs,
        }) = heap.pop()
        {
            match &self.nodes[node as usize] {
                Node::Leaf { start, end } => {
                    for &pi in &self.order[*start as usize..*end as usize] {
                        out.push(pi);
                    }
                    leaves_taken += 1;
                    if leaves_taken >= top_l {
                        return;
                    }
                }
                Node::Split {
                    dim,
                    val,
                    left,
                    right,
                } => {
                    let d = *dim as usize;
                    let qd = q[d];
                    let (near, far) = if qd < *val {
                        (*left, *right)
                    } else {
                        (*right, *left)
                    };
                    let bound = f64::from_bits(bound_bits);
                    let new_off = qd - *val;
                    let far_bound = (bound - offs[d] * offs[d] + new_off * new_off).max(0.0);
                    let mut far_offs = offs.clone();
                    far_offs[d] = new_off;
                    heap.push(Visit {
                        bound_bits: far_bound.to_bits(),
                        node: far,
                        offs: far_offs,
                    });
                    heap.push(Visit {
                        bound_bits,
                        node: near,
                        offs,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_points(n: usize) -> Vec<f64> {
        // n points on a 2-d spiral-ish deterministic layout.
        let mut c = Vec::with_capacity(n * 2);
        for i in 0..n {
            c.push(((i * 13) % 97) as f64);
            c.push(((i * 29) % 89) as f64);
        }
        c
    }

    #[test]
    fn exact_nearest_is_inside_top_leaves() {
        let n = 300;
        let coords = grid_points(n);
        let tree = KdTree::build(coords.clone(), 2);
        assert_eq!(tree.len(), n);
        let mut out = Vec::new();
        for probe in 0..20 {
            let q = [
                ((probe * 7) % 97) as f64 + 0.3,
                ((probe * 11) % 89) as f64 - 0.4,
            ];
            // Brute-force nearest.
            let mut best = 0u32;
            let mut best_d = f64::INFINITY;
            for i in 0..n {
                let dx = coords[i * 2] - q[0];
                let dy = coords[i * 2 + 1] - q[1];
                let d = dx * dx + dy * dy;
                if d < best_d {
                    best_d = d;
                    best = i as u32;
                }
            }
            tree.query_leaves(&q, 8, &mut out);
            assert!(
                out.contains(&best),
                "probe {probe}: true nearest {best} missing from top-8 leaves"
            );
        }
    }

    #[test]
    fn queries_are_pure_and_repeatable() {
        let tree = KdTree::build(grid_points(120), 2);
        let q = [41.5, 12.5];
        let mut a = Vec::new();
        let mut b = Vec::new();
        tree.query_leaves(&q, 4, &mut a);
        tree.query_leaves(&q, 4, &mut b);
        assert_eq!(a, b, "identical queries must rebuild identical sets");
        assert!(!a.is_empty());
    }

    #[test]
    fn empty_tree_yields_no_candidates() {
        let tree = KdTree::build(Vec::new(), 16);
        let mut out = vec![7u32];
        tree.query_leaves(&[0.0; 16], 8, &mut out);
        assert!(out.is_empty());
    }
}
