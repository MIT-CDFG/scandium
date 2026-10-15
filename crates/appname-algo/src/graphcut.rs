// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Own-written push-relabel max-flow/min-cut from the published algorithm
// (A. V. Goldberg, R. E. Tarjan, "A new approach to the maximum flow
// problem", JACM 1988 — era-expired lineage recorded in census row
// B1-ortools-maxflow.yaml). The research-licensed maxflow-v3.x code was
// NOT fetched, NOT read, NOT used; the OR-Tools tree is pin-recorded only
// and was likewise never staged or opened — no expression from either is
// present here. boykov04 is cited for the graph-cut energy-minimization
// FRAMING only (register P-6744923 "Expired - Lifetime" covers the
// multi-label alpha-expansion use on that verified-expiry basis).

//! Spec: ALGORITHMS.md §8 `legacy_selection` (max-flow kernel shared with
//! the §3/§5 boundary cuts); ES §39.2/§39.4 selection tools, ES §45.3
//! content-aware-fill boundary-cut seams (the in-plan reuse consumer).
//! Panorama compositing cuts are ALGORITHMS.md §22 — not an ES anchor:
//! the current ES records panorama stitch as post-1.0 plugin territory
//! with no core reservation. INTEGRATION-NOTES §6 order 8
//! (`ortools-maxflow` slot).
//!
//! Deterministic min-cut/max-flow engine for graph-cut selection and
//! boundary-cut compositing:
//!
//! * general node/arc API — arbitrary directed arcs with per-direction
//!   capacities plus implicit source/sink terminals — so the same engine
//!   serves GrabCut t/n-link grids, §45.3 content-aware-fill boundary-cut
//!   compositing, and any later pano compositing cuts (ALGORITHMS.md §22;
//!   post-1.0 plugin territory per the ES);
//! * FIFO push-relabel with exact-distance global relabeling (both
//!   heuristics deterministic: fixed arc-insertion adjacency order, queue
//!   rebuilds scan nodes in index order, no hash-map iteration, single
//!   thread);
//! * min-cut extraction as the source-side reachable set of the FINAL
//!   residual graph (the census-recorded route), available after
//!   [`CutGraph::solve`].
//!
//! Grid consumers map pixel `(x, y)` to node `y * w + x` and get
//! CSR-packed adjacency in insertion order — build arcs in raster order
//! and the memory layout is grid-coherent. Capacities are `f64`; the
//! solver is exact for integer-valued capacities and floating-capacity
//! deterministic (fixed operation order) otherwise.

/// Max-flow/min-cut solver over a directed graph with implicit
/// source/sink terminals.
///
/// Lifecycle: [`CutGraph::new`] → [`CutGraph::add_edge`] /
/// [`CutGraph::add_terminal`] (any order, insertion order is the
/// deterministic adjacency order) → [`CutGraph::solve`] →
/// [`CutGraph::source_side`].
pub struct CutGraph {
    /// Non-terminal node count; terminals are appended internally as
    /// `node_count` (source) and `node_count + 1` (sink).
    node_count: usize,
    /// Arc tails, paired storage: arc `a` and `a ^ 1` are a
    /// forward/reverse pair.
    tails: Vec<u32>,
    /// Arc heads, paired with `tails`.
    heads: Vec<u32>,
    /// Residual capacities (mutated by the solve).
    res: Vec<f64>,
    /// CSR adjacency offsets (built by `solve`), length `N + 1`.
    adj_start: Vec<usize>,
    /// CSR adjacency arc ids in per-node insertion order.
    adj_arcs: Vec<u32>,
    /// Set once `solve` has run.
    solved: bool,
}

impl CutGraph {
    /// New empty graph over `node_count` non-terminal nodes.
    pub fn new(node_count: usize) -> CutGraph {
        CutGraph {
            node_count,
            tails: Vec::new(),
            heads: Vec::new(),
            res: Vec::new(),
            adj_start: Vec::new(),
            adj_arcs: Vec::new(),
            solved: false,
        }
    }

    /// Pre-size arc storage (`arc_pairs` = expected `add_edge` +
    /// `add_terminal` calls); a grid consumer passes the n-link + t-link
    /// count for one contiguous allocation.
    pub fn with_capacity(node_count: usize, arc_pairs: usize) -> CutGraph {
        let mut g = CutGraph::new(node_count);
        g.tails.reserve(arc_pairs * 2);
        g.heads.reserve(arc_pairs * 2);
        g.res.reserve(arc_pairs * 2);
        g
    }

    #[inline]
    fn source(&self) -> u32 {
        self.node_count as u32
    }

    #[inline]
    fn sink(&self) -> u32 {
        self.node_count as u32 + 1
    }

    fn push_arc_pair(&mut self, u: u32, v: u32, cap_uv: f64, cap_vu: f64) {
        debug_assert!(cap_uv >= 0.0 && cap_vu >= 0.0, "negative capacity");
        self.tails.push(u);
        self.heads.push(v);
        self.res.push(cap_uv);
        self.tails.push(v);
        self.heads.push(u);
        self.res.push(cap_vu);
    }

    /// Add a directed arc pair between non-terminal nodes `u` and `v`
    /// with capacities `cap_uv` (u→v) and `cap_vu` (v→u). An undirected
    /// n-link is `add_edge(u, v, w, w)`. Parallel arcs are allowed.
    ///
    /// # Panics
    ///
    /// Panics if `u` or `v` is out of range (≥ the node count) or if
    /// called after [`CutGraph::solve`]. Negative capacities are a
    /// debug-assertion failure.
    pub fn add_edge(&mut self, u: usize, v: usize, cap_uv: f64, cap_vu: f64) {
        assert!(
            u < self.node_count && v < self.node_count,
            "node out of range"
        );
        assert!(!self.solved, "graph already solved");
        self.push_arc_pair(u as u32, v as u32, cap_uv, cap_vu);
    }

    /// Add terminal capacities for node `u`: `cap_source` on the
    /// source→u arc and `cap_sink` on the u→sink arc (t-links). Repeated
    /// calls accumulate as parallel arcs.
    ///
    /// # Panics
    ///
    /// Panics if `u` is out of range (≥ the node count) or if called
    /// after [`CutGraph::solve`]. Negative capacities are a
    /// debug-assertion failure.
    pub fn add_terminal(&mut self, u: usize, cap_source: f64, cap_sink: f64) {
        assert!(u < self.node_count, "node out of range");
        assert!(!self.solved, "graph already solved");
        if cap_source > 0.0 {
            let (s, u32u) = (self.source(), u as u32);
            self.push_arc_pair(s, u32u, cap_source, 0.0);
        }
        if cap_sink > 0.0 {
            let (t, u32u) = (self.sink(), u as u32);
            self.push_arc_pair(u32u, t, cap_sink, 0.0);
        }
    }

    /// Build the CSR adjacency in per-node arc-insertion order
    /// (deterministic).
    fn build_adjacency(&mut self) {
        let n_total = self.node_count + 2;
        let mut counts = vec![0usize; n_total + 1];
        for &t in &self.tails {
            counts[t as usize + 1] += 1;
        }
        for i in 0..n_total {
            counts[i + 1] += counts[i];
        }
        self.adj_start = counts.clone();
        self.adj_arcs = vec![0u32; self.tails.len()];
        let mut cursor = counts;
        for (a, &t) in self.tails.iter().enumerate() {
            self.adj_arcs[cursor[t as usize]] = a as u32;
            cursor[t as usize] += 1;
        }
    }

    /// Exact-distance global relabeling: heights = residual BFS distance
    /// to the sink; nodes that cannot reach the sink get
    /// `N + (residual distance from the source)`; the source is pinned at
    /// `N`. Both BFS passes visit adjacency in insertion order (FIFO
    /// queue), so the result is deterministic.
    fn global_relabel(&self, height: &mut [u32]) {
        let n_total = self.node_count + 2;
        let unreached = 2 * n_total as u32;
        let source = self.source() as usize;
        let sink = self.sink() as usize;
        for h in height.iter_mut() {
            *h = unreached;
        }
        // Pass 1: distance to sink over reverse residual arcs.
        height[sink] = 0;
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(sink as u32);
        while let Some(v) = queue.pop_front() {
            let v = v as usize;
            for &a in &self.adj_arcs[self.adj_start[v]..self.adj_start[v + 1]] {
                // Arc a: v -> w. Its pair a^1 is w -> v; if that pair has
                // residual capacity, w can step to v in the residual graph.
                let w = self.heads[a as usize] as usize;
                if w != source && height[w] == unreached && self.res[(a ^ 1) as usize] > 0.0 {
                    height[w] = height[v] + 1;
                    queue.push_back(w as u32);
                }
            }
        }
        // Pass 2: N + distance from source (excess-return heights for
        // nodes cut off from the sink).
        height[source] = n_total as u32;
        queue.push_back(source as u32);
        while let Some(v) = queue.pop_front() {
            let v = v as usize;
            for &a in &self.adj_arcs[self.adj_start[v]..self.adj_start[v + 1]] {
                let w = self.heads[a as usize] as usize;
                if height[w] == unreached && self.res[(a ^ 1) as usize] > 0.0 {
                    height[w] = height[v] + 1;
                    queue.push_back(w as u32);
                }
            }
        }
    }

    /// Run FIFO push-relabel to a full maximum flow. Returns the max-flow
    /// value (== min-cut capacity). Idempotent: a second call returns the
    /// stored value.
    pub fn solve(&mut self) -> f64 {
        if self.solved {
            return self.flow_value();
        }
        self.build_adjacency();
        let n_total = self.node_count + 2;
        let source = self.source() as usize;
        let sink = self.sink() as usize;
        let height_cap = 2 * n_total as u32;

        let mut height = vec![0u32; n_total];
        let mut excess = vec![0.0f64; n_total];
        let mut current = vec![0usize; n_total];
        let mut in_queue = vec![false; n_total];
        let mut queue = std::collections::VecDeque::new();

        self.global_relabel(&mut height);

        // Saturate all source arcs (insertion order).
        for ai in self.adj_start[source]..self.adj_start[source + 1] {
            let a = self.adj_arcs[ai] as usize;
            let delta = self.res[a];
            if delta > 0.0 {
                let v = self.heads[a] as usize;
                self.res[a] = 0.0;
                self.res[a ^ 1] += delta;
                excess[v] += delta;
                if v != sink && !in_queue[v] {
                    in_queue[v] = true;
                    queue.push_back(v as u32);
                }
            }
        }

        // Periodic global relabel cadence (deterministic counter).
        let relabel_period = n_total.max(16);
        let mut relabels_since_global = 0usize;

        while let Some(u) = queue.pop_front() {
            let u = u as usize;
            in_queue[u] = false;
            // Discharge u.
            'discharge: while excess[u] > 0.0 {
                let begin = self.adj_start[u];
                let end = self.adj_start[u + 1];
                let mut ai = begin + current[u];
                while ai < end {
                    let a = self.adj_arcs[ai] as usize;
                    let v = self.heads[a] as usize;
                    if self.res[a] > 0.0 && height[u] == height[v] + 1 {
                        let delta = excess[u].min(self.res[a]);
                        self.res[a] -= delta;
                        self.res[a ^ 1] += delta;
                        excess[u] -= delta;
                        excess[v] += delta;
                        if v != source && v != sink && !in_queue[v] {
                            in_queue[v] = true;
                            queue.push_back(v as u32);
                        }
                        if excess[u] <= 0.0 {
                            current[u] = ai - begin;
                            break 'discharge;
                        }
                    }
                    ai += 1;
                }
                // Current-arc list exhausted: relabel u.
                let mut min_h = u32::MAX;
                for aj in begin..end {
                    let a = self.adj_arcs[aj] as usize;
                    if self.res[a] > 0.0 {
                        min_h = min_h.min(height[self.heads[a] as usize]);
                    }
                }
                if min_h == u32::MAX || min_h + 1 >= height_cap {
                    // No residual arc (isolated excess: only possible with
                    // zero-capacity construction) or height cap reached —
                    // deactivate.
                    height[u] = height_cap;
                    break 'discharge;
                }
                height[u] = min_h + 1;
                current[u] = 0;
                relabels_since_global += 1;
                if relabels_since_global >= relabel_period {
                    relabels_since_global = 0;
                    self.global_relabel(&mut height);
                    for c in current.iter_mut() {
                        *c = 0;
                    }
                    // Rebuild the active queue in node-index order
                    // (deterministic).
                    queue.clear();
                    for w in in_queue.iter_mut() {
                        *w = false;
                    }
                    for v in 0..n_total {
                        if v != source && v != sink && excess[v] > 0.0 && height[v] < height_cap {
                            in_queue[v] = true;
                            queue.push_back(v as u32);
                        }
                    }
                    if height[u] >= height_cap || !in_queue[u] {
                        break 'discharge;
                    }
                    // u stays active; it was re-queued above — stop this
                    // discharge and let the queue order take over.
                    break 'discharge;
                }
            }
            if excess[u] > 0.0 && height[u] < height_cap && !in_queue[u] {
                in_queue[u] = true;
                queue.push_back(u as u32);
            }
        }

        self.solved = true;
        self.flow_value()
    }

    /// Max-flow value = net residual gain on the reverse arcs into the
    /// sink (valid after [`CutGraph::solve`]).
    fn flow_value(&self) -> f64 {
        let sink = self.sink() as usize;
        let mut total = 0.0;
        for &a in &self.adj_arcs[self.adj_start[sink]..self.adj_start[sink + 1]] {
            // Arcs tailed at the sink are reverse arcs of t-links; their
            // residual capacity equals the flow pushed into the sink.
            let a = a as usize;
            if self.tails[a ^ 1] != self.sink() {
                total += self.res[a];
            }
        }
        total
    }

    /// Min-cut extraction: the source-side reachable set of the final
    /// residual graph (census-recorded route). Returns one flag per
    /// non-terminal node; `true` = source side. Panics if called before
    /// [`CutGraph::solve`].
    pub fn source_side(&self) -> Vec<bool> {
        assert!(self.solved, "source_side requires a completed solve");
        let n_total = self.node_count + 2;
        let source = self.source() as usize;
        let mut visited = vec![false; n_total];
        visited[source] = true;
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(source as u32);
        while let Some(v) = queue.pop_front() {
            let v = v as usize;
            for &a in &self.adj_arcs[self.adj_start[v]..self.adj_start[v + 1]] {
                let a = a as usize;
                let w = self.heads[a] as usize;
                if !visited[w] && self.res[a] > 0.0 {
                    visited[w] = true;
                    queue.push_back(w as u32);
                }
            }
        }
        debug_assert!(
            !visited[self.sink() as usize],
            "sink reachable after solve — flow not maximum"
        );
        visited.truncate(self.node_count);
        visited
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::SplitMix64;

    /// Cut capacity of the partition `src_side` (terminal + crossing
    /// arcs), computed from original capacities — the oracle both the
    /// brute-force cross-check and the extraction test share.
    fn cut_capacity(
        node_count: usize,
        edges: &[(usize, usize, f64, f64)],
        terminals: &[(usize, f64, f64)],
        src_side: &[bool],
    ) -> f64 {
        let mut cap = 0.0;
        for &(u, cs, ct) in terminals {
            if !src_side[u] {
                cap += cs; // source -> u crosses
            } else {
                cap += ct; // u -> sink crosses
            }
        }
        for &(u, v, cuv, cvu) in edges {
            if src_side[u] && !src_side[v] {
                cap += cuv;
            }
            if src_side[v] && !src_side[u] {
                cap += cvu;
            }
        }
        let _ = node_count;
        cap
    }

    fn build(
        node_count: usize,
        edges: &[(usize, usize, f64, f64)],
        terminals: &[(usize, f64, f64)],
    ) -> CutGraph {
        let mut g = CutGraph::with_capacity(node_count, edges.len() + terminals.len());
        for &(u, v, cuv, cvu) in edges {
            g.add_edge(u, v, cuv, cvu);
        }
        for &(u, cs, ct) in terminals {
            g.add_terminal(u, cs, ct);
        }
        g
    }

    #[test]
    fn cut_hand_computed_small() {
        // Two-node classic: source->a 3, source->b 2, a->b 1, a->sink 2,
        // b->sink 3. Max flow = 5 (a: 2 direct + 1 via b; b: 2 direct).
        let edges = [(0usize, 1usize, 1.0f64, 0.0f64)];
        let terminals = [(0usize, 3.0f64, 2.0f64), (1usize, 2.0f64, 3.0f64)];
        let mut g = build(2, &edges, &terminals);
        let flow = g.solve();
        assert!((flow - 5.0).abs() < 1e-12, "flow {flow} != 5");
        let side = g.source_side();
        let cap = cut_capacity(2, &edges, &terminals, &side);
        assert!(
            (cap - flow).abs() < 1e-12,
            "extracted cut {cap} != flow {flow}"
        );
    }

    #[test]
    fn cut_grid_bottleneck() {
        // 4x3 grid, columns 0/3 pinned to source/sink with cap 100,
        // horizontal n-links 1.5, vertical 1.5. Every vertical cut
        // boundary severs 3 horizontal links: min cut = 4.5.
        let (w, h) = (4usize, 3usize);
        let mut edges = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let n = y * w + x;
                if x + 1 < w {
                    edges.push((n, n + 1, 1.5, 1.5));
                }
                if y + 1 < h {
                    edges.push((n, n + w, 1.5, 1.5));
                }
            }
        }
        let mut terminals = Vec::new();
        for y in 0..h {
            terminals.push((y * w, 100.0, 0.0));
            terminals.push((y * w + w - 1, 0.0, 100.0));
        }
        let mut g = build(w * h, &edges, &terminals);
        let flow = g.solve();
        assert!((flow - 4.5).abs() < 1e-12, "grid flow {flow} != 4.5");
        let side = g.source_side();
        for y in 0..h {
            assert!(side[y * w], "source column left the source side");
            assert!(!side[y * w + w - 1], "sink column on the source side");
        }
        let cap = cut_capacity(w * h, &edges, &terminals, &side);
        assert!((cap - flow).abs() < 1e-12);
    }

    /// Brute-force min-cut over all 2^n partitions of tiny fixed-seed
    /// random graphs (integer-valued capacities so equality is exact
    /// up to f64 rounding of small sums).
    #[test]
    fn cut_matches_brute_force_min_cut() {
        let n = 6usize;
        let mut rng = SplitMix64(0x0891_0891_0891_0891);
        for _case in 0..24 {
            let mut edges = Vec::new();
            for u in 0..n {
                for v in (u + 1)..n {
                    if rng.next_f64() < 0.55 {
                        let cuv = (rng.next_u64() % 8) as f64;
                        let cvu = (rng.next_u64() % 8) as f64;
                        edges.push((u, v, cuv, cvu));
                    }
                }
            }
            let mut terminals = Vec::new();
            for u in 0..n {
                let cs = (rng.next_u64() % 10) as f64;
                let ct = (rng.next_u64() % 10) as f64;
                terminals.push((u, cs, ct));
            }
            let mut g = build(n, &edges, &terminals);
            let flow = g.solve();
            let mut best = f64::INFINITY;
            for mask in 0u32..(1 << n) {
                let side: Vec<bool> = (0..n).map(|i| mask & (1 << i) != 0).collect();
                best = best.min(cut_capacity(n, &edges, &terminals, &side));
            }
            assert!(
                (flow - best).abs() < 1e-9,
                "flow {flow} != brute-force min cut {best}"
            );
            let side = g.source_side();
            let cap = cut_capacity(n, &edges, &terminals, &side);
            assert!(
                (cap - best).abs() < 1e-9,
                "extracted cut {cap} != brute-force min cut {best}"
            );
        }
    }

    /// Flow conservation + capacity constraints on a fixed-seed graph:
    /// residuals stay in [0, cap_fwd + cap_rev], net outflow is zero at
    /// every interior node, and net inflow at the sink equals the
    /// reported flow value.
    #[test]
    fn cut_flow_conservation_and_capacity() {
        let n = 12usize;
        let mut rng = SplitMix64(0xB10C_CA81_0000_0001);
        let mut edges = Vec::new();
        for u in 0..n {
            for v in (u + 1)..n {
                if rng.next_f64() < 0.4 {
                    edges.push((
                        u,
                        v,
                        (rng.next_u64() % 9) as f64,
                        (rng.next_u64() % 9) as f64,
                    ));
                }
            }
        }
        let mut terminals = Vec::new();
        for u in 0..n {
            terminals.push((
                u,
                (rng.next_u64() % 12) as f64,
                (rng.next_u64() % 12) as f64,
            ));
        }
        let mut g = build(n, &edges, &terminals);
        let cap0 = g.res.clone();
        let flow = g.solve();

        let n_total = n + 2;
        let mut net = vec![0.0f64; n_total];
        for a in (0..g.res.len()).step_by(2) {
            assert!(g.res[a] >= 0.0, "negative residual on arc {a}");
            assert!(g.res[a ^ 1] >= 0.0, "negative residual on arc {}", a ^ 1);
            // Physical net flow across the pair, counted once: forward
            // consumption minus reverse consumption.
            let f = (cap0[a] - g.res[a] - (cap0[a ^ 1] - g.res[a ^ 1])) * 0.5;
            net[g.tails[a] as usize] += f;
            net[g.heads[a] as usize] -= f;
        }
        // Index loop kept: the assertion references net[u] by node index.
        #[allow(clippy::needless_range_loop)]
        for u in 0..n {
            assert!(
                net[u].abs() < 1e-9,
                "conservation violated at node {u}: {}",
                net[u]
            );
        }
        assert!((net[n] - flow).abs() < 1e-9, "source outflow != flow");
        assert!((-net[n + 1] - flow).abs() < 1e-9, "sink inflow != flow");
        // Max-flow == min-cut capacity for the extracted partition.
        let side = g.source_side();
        let cap = cut_capacity(n, &edges, &terminals, &side);
        assert!((cap - flow).abs() < 1e-9, "cut {cap} != flow {flow}");
    }

    #[test]
    fn cut_determinism_two_runs() {
        let n = 10usize;
        let mk = || {
            let mut rng = SplitMix64(0xDE7E_0891_0000_0002);
            let mut edges = Vec::new();
            for u in 0..n {
                for v in (u + 1)..n {
                    if rng.next_f64() < 0.5 {
                        edges.push((u, v, rng.next_f64() * 4.0, rng.next_f64() * 4.0));
                    }
                }
            }
            let mut terminals = Vec::new();
            for u in 0..n {
                terminals.push((u, rng.next_f64() * 6.0, rng.next_f64() * 6.0));
            }
            build(n, &edges, &terminals)
        };
        let mut a = mk();
        let mut b = mk();
        let fa = a.solve();
        let fb = b.solve();
        assert_eq!(fa.to_bits(), fb.to_bits(), "flow value differs across runs");
        assert_eq!(
            a.source_side(),
            b.source_side(),
            "cut side differs across runs"
        );
        assert_eq!(a.res, b.res, "residual graph differs across runs");
    }

    #[test]
    fn cut_zero_capacity_arcs_are_inert() {
        // Zero-capacity arcs (and zero-capacity add_terminal calls,
        // which push nothing) must not change the flow value or the
        // extracted cut vs the same graph without them.
        let edges = [(0usize, 1usize, 2.0f64, 0.0f64)];
        let terminals = [(0usize, 3.0f64, 0.0f64), (1usize, 0.0f64, 4.0f64)];
        let mut plain = build(3, &edges, &terminals);
        let f_plain = plain.solve();

        let mut padded = CutGraph::new(3);
        padded.add_edge(0, 1, 2.0, 0.0);
        padded.add_edge(0, 2, 0.0, 0.0); // zero-capacity clutter
        padded.add_edge(2, 1, 0.0, 0.0);
        padded.add_terminal(0, 3.0, 0.0);
        padded.add_terminal(1, 0.0, 4.0);
        padded.add_terminal(2, 0.0, 0.0); // pushes no arcs at all
        let f_padded = padded.solve();

        assert_eq!(f_plain, 2.0, "plain chain flow");
        assert_eq!(f_padded, 2.0, "zero-cap arcs changed the flow value");
        let side = padded.source_side();
        assert_eq!(side, plain.source_side(), "zero-cap arcs changed the cut");
        // Source cap 3 > bottleneck 2: node 0 stays source-reachable;
        // node 2 hangs off zero-capacity arcs only — sink side.
        assert_eq!(side, vec![true, false, false]);

        // All-zero graph: flow 0, nothing source-reachable.
        let mut zero = CutGraph::new(2);
        zero.add_edge(0, 1, 0.0, 0.0);
        zero.add_terminal(0, 0.0, 0.0);
        zero.add_terminal(1, 0.0, 0.0);
        assert_eq!(zero.solve(), 0.0);
        assert_eq!(zero.source_side(), vec![false, false]);
    }

    #[test]
    fn cut_disconnected_components_zero_flow() {
        // Source-only island {0, 1}, sink-only island {2, 3}, fully
        // isolated node 4: no source→sink path exists, so flow = 0 and
        // the extracted partition must ALSO have zero cut capacity
        // (source island source-side, everything else sink-side).
        let edges = [(0usize, 1usize, 5.0f64, 5.0f64), (2, 3, 5.0, 5.0)];
        let terminals = [(0usize, 7.0f64, 0.0f64), (2usize, 0.0f64, 6.0f64)];
        let mut g = build(5, &edges, &terminals);
        let flow = g.solve();
        assert_eq!(flow, 0.0, "disconnected graph must carry zero flow");
        let side = g.source_side();
        // Pushed excess must fully drain back: the source t-link's
        // residual is restored, keeping the island source-reachable.
        assert_eq!(
            side,
            vec![true, true, false, false, false],
            "source island must stay source-side; sink island + isolated node sink-side"
        );
        assert_eq!(cut_capacity(5, &edges, &terminals, &side), 0.0);
    }

    #[test]
    fn cut_parallel_and_antiparallel_arcs_accumulate() {
        // Three parallel arcs ≡ one summed arc.
        let terminals = [(0usize, 10.0f64, 0.0f64), (1usize, 0.0f64, 10.0f64)];
        let mut many = CutGraph::new(2);
        for _ in 0..3 {
            many.add_edge(0, 1, 1.5, 0.5);
        }
        for &(u, cs, ct) in &terminals {
            many.add_terminal(u, cs, ct);
        }
        let mut one = build(2, &[(0, 1, 4.5, 1.5)], &terminals);
        let fm = many.solve();
        assert_eq!(fm, one.solve(), "parallel arcs must accumulate");
        assert_eq!(fm, 4.5);
        assert_eq!(many.source_side(), one.source_side());

        // Anti-parallel pairs: add_edge(0,1,a,b) + add_edge(1,0,c,d)
        // must behave as capacities u→v = a + d, v→u = b + c.
        let anti_edges = [(0usize, 1usize, 2.0f64, 1.0f64), (1, 0, 3.0, 0.5)];
        let mut anti = build(2, &anti_edges, &terminals);
        let mut merged = build(2, &[(0, 1, 2.5, 4.0)], &terminals);
        let fa = anti.solve();
        assert_eq!(fa, merged.solve(), "anti-parallel pair mismatch (u→v)");
        assert_eq!(fa, 2.5);
        // Reverse direction: pin the terminals the other way round.
        let rev_terminals = [(1usize, 10.0f64, 0.0f64), (0usize, 0.0f64, 10.0f64)];
        let mut anti_r = build(2, &anti_edges, &rev_terminals);
        assert_eq!(anti_r.solve(), 4.0, "anti-parallel pair mismatch (v→u)");
    }

    #[test]
    fn cut_bottleneck_at_source_vs_sink_side() {
        // Chain source →(1)→ 0 →(10)→ 1 →(10)→ sink: the unique min cut
        // is the saturated SOURCE arc, so no node is source-reachable.
        let mut at_source = CutGraph::new(2);
        at_source.add_edge(0, 1, 10.0, 0.0);
        at_source.add_terminal(0, 1.0, 0.0);
        at_source.add_terminal(1, 0.0, 10.0);
        assert_eq!(at_source.solve(), 1.0);
        assert_eq!(
            at_source.source_side(),
            vec![false, false],
            "source-side cut must leave every node on the sink side"
        );

        // Mirror chain with the bottleneck at the SINK arc: every node
        // stays source-reachable.
        let mut at_sink = CutGraph::new(2);
        at_sink.add_edge(0, 1, 10.0, 0.0);
        at_sink.add_terminal(0, 10.0, 0.0);
        at_sink.add_terminal(1, 0.0, 1.0);
        assert_eq!(at_sink.solve(), 1.0);
        assert_eq!(
            at_sink.source_side(),
            vec![true, true],
            "sink-side cut must keep every node on the source side"
        );
    }

    /// Max-flow VALUE invariance under arc-insertion-order permutation,
    /// on integer-capacity graphs (all arithmetic stays exact, so
    /// equality is bitwise). What determinism promises here: the VALUE
    /// is insertion-order invariant; the cut SET is NOT promised across
    /// different insertion orders (distinct min cuts can tie, and the
    /// adjacency order steers which one the solver lands on) — each
    /// permutation's cut is held only to the cut-capacity == flow
    /// oracle. Bit-identical EVERYTHING is promised only for the SAME
    /// insertion order, covered by `cut_determinism_two_runs`.
    #[test]
    fn cut_flow_value_invariant_under_insertion_order() {
        let n = 8usize;
        let mut rng = SplitMix64(0x0891_5EED_0000_0003);
        for _case in 0..10 {
            let mut edges = Vec::new();
            for u in 0..n {
                for v in (u + 1)..n {
                    if rng.next_f64() < 0.5 {
                        edges.push((
                            u,
                            v,
                            (rng.next_u64() % 7) as f64,
                            (rng.next_u64() % 7) as f64,
                        ));
                    }
                }
            }
            let mut terminals = Vec::new();
            for u in 0..n {
                terminals.push((u, (rng.next_u64() % 9) as f64, (rng.next_u64() % 9) as f64));
            }
            let mut base = build(n, &edges, &terminals);
            let base_flow = base.solve();
            let base_side = base.source_side();
            assert!((cut_capacity(n, &edges, &terminals, &base_side) - base_flow).abs() < 1e-9);
            for _perm in 0..4 {
                let mut e2 = edges.clone();
                let mut t2 = terminals.clone();
                // Deterministic Fisher–Yates off the shared seeded stream.
                for i in (1..e2.len()).rev() {
                    e2.swap(i, (rng.next_u64() % (i as u64 + 1)) as usize);
                }
                for i in (1..t2.len()).rev() {
                    t2.swap(i, (rng.next_u64() % (i as u64 + 1)) as usize);
                }
                let mut g = build(n, &e2, &t2);
                let flow = g.solve();
                assert_eq!(
                    flow.to_bits(),
                    base_flow.to_bits(),
                    "integer-capacity flow value must be insertion-order invariant"
                );
                let side = g.source_side();
                assert!(
                    (cut_capacity(n, &e2, &t2, &side) - flow).abs() < 1e-9,
                    "permuted-order extracted cut is not a min cut"
                );
            }
        }
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "negative capacity")]
    fn cut_nan_edge_capacity_debug_asserts() {
        // Documents current behavior, not a contract: NaN fails the
        // `cap >= 0` debug assertion (debug builds only; a release
        // build would carry the NaN into the solve unchecked).
        let mut g = CutGraph::new(2);
        g.add_edge(0, 1, f64::NAN, 0.0);
    }

    #[test]
    fn cut_nan_terminal_capacity_is_dropped() {
        // Documents current behavior, not a contract: `add_terminal`
        // keeps an arc only when `cap > 0.0`, which is false for NaN —
        // NaN t-links are silently dropped and the solve sees an empty
        // graph (flow 0, nothing source-reachable).
        let mut g = CutGraph::new(1);
        g.add_terminal(0, f64::NAN, f64::NAN);
        assert_eq!(g.solve(), 0.0);
        assert_eq!(g.source_side(), vec![false]);
    }
}
