// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Portions ported from PyMatting (https://github.com/pymatting/pymatting,
// commit fcda1e52fdeb47d03ec2478c1aeafd0b7bd387de, vendored at
// appname-proto/vendor/pymatting/): pymatting/solver/cg.py and
// pymatting/preconditioner/ichol.py.
// MIT License, Copyright (c) 2020 PyMatting. License text carried in
// THIRD-PARTY-NOTICES.md and vendor/pymatting/LICENSE.md.

//! Spec: ES §32.6 / §39.4 (Select & Mask matting core); ALGORITHMS.md §7
//! (`alpha_matting`, closed-form primary, basis levin08); INTEGRATION-NOTES
//! §6 order 1 (`pymatting` port).
//!
//! Sparse linear-algebra substrate for the matting engines: CSR/CSC storage,
//! a preconditioned conjugate-gradient solver (port of pymatting `cg.py`),
//! and the thresholded incomplete-Cholesky preconditioner (port of
//! pymatting `ichol.py`, linked-list column algorithm after Jones & Plassmann
//! 1995).
//!
//! Determinism: fixed iteration order, no threading, no hash-order
//! dependence; identical inputs produce bit-identical factors and solutions.

/// Compressed sparse row matrix (square, f64).
#[derive(Clone, Debug)]
pub struct Csr {
    /// Row count == column count.
    pub n: usize,
    /// Row pointers, length `n + 1`.
    pub indptr: Vec<usize>,
    /// Column indices per stored entry.
    pub indices: Vec<usize>,
    /// Stored entry values.
    pub values: Vec<f64>,
}

impl Csr {
    /// `y = A x`.
    pub fn matvec(&self, x: &[f64], y: &mut [f64]) {
        assert_eq!(x.len(), self.n);
        assert_eq!(y.len(), self.n);
        for (i, yv) in y.iter_mut().enumerate().take(self.n) {
            let mut s = 0.0;
            for k in self.indptr[i]..self.indptr[i + 1] {
                s += self.values[k] * x[self.indices[k]];
            }
            *yv = s;
        }
    }

    /// Submatrix `A[rows, :][:, cols]` where `rows`/`cols` are boolean masks
    /// over 0..n (the pymatting `L[is_unknown][:, is_unknown]` split used by
    /// `estimate_alpha_cf`). Returns a CSR over the selected index sets with
    /// their natural (ascending) order.
    pub fn submatrix(&self, rows: &[bool], cols: &[bool]) -> Csr {
        assert_eq!(rows.len(), self.n);
        assert_eq!(cols.len(), self.n);
        // Old-column -> new-column map (usize::MAX = dropped).
        let mut col_map = vec![usize::MAX; self.n];
        let mut nc = 0usize;
        for (j, &keep) in cols.iter().enumerate() {
            if keep {
                col_map[j] = nc;
                nc += 1;
            }
        }
        let nr = rows.iter().filter(|&&k| k).count();
        assert_eq!(
            nr, nc,
            "submatrix() here is used square (masked principal blocks)"
        );
        let mut indptr = Vec::with_capacity(nr + 1);
        let mut indices = Vec::new();
        let mut values = Vec::new();
        indptr.push(0);
        for (i, &keep) in rows.iter().enumerate().take(self.n) {
            if !keep {
                continue;
            }
            for k in self.indptr[i]..self.indptr[i + 1] {
                let j = self.indices[k];
                if col_map[j] != usize::MAX {
                    indices.push(col_map[j]);
                    values.push(self.values[k]);
                }
            }
            indptr.push(indices.len());
        }
        Csr {
            n: nr,
            indptr,
            indices,
            values,
        }
    }

    /// `y = A[rows, :][:, cols] x` for a rectangular masked block (used for
    /// the `R.dot(m)` right-hand-side term without materializing R).
    pub fn masked_rect_matvec(
        &self,
        rows: &[bool],
        cols: &[bool],
        x_by_old_col: &[f64],
        y: &mut Vec<f64>,
    ) {
        y.clear();
        for (i, &keep) in rows.iter().enumerate().take(self.n) {
            if !keep {
                continue;
            }
            let mut s = 0.0;
            for k in self.indptr[i]..self.indptr[i + 1] {
                let j = self.indices[k];
                if cols[j] {
                    s += self.values[k] * x_by_old_col[j];
                }
            }
            y.push(s);
        }
    }

    /// CSR -> CSC of the same matrix. For the symmetric matting Laplacian the
    /// result equals the CSR of Aᵀ = A reinterpreted column-wise, which is
    /// what `ichol` consumes (pymatting passes `A.T` for CSR inputs).
    pub fn to_csc(&self) -> Csc {
        let mut counts = vec![0usize; self.n + 1];
        for &j in &self.indices {
            counts[j + 1] += 1;
        }
        for j in 0..self.n {
            counts[j + 1] += counts[j];
        }
        let mut indptr = counts.clone();
        let mut indices = vec![0usize; self.indices.len()];
        let mut values = vec![0f64; self.values.len()];
        for i in 0..self.n {
            for k in self.indptr[i]..self.indptr[i + 1] {
                let j = self.indices[k];
                let dst = indptr[j];
                indices[dst] = i;
                values[dst] = self.values[k];
                indptr[j] += 1;
            }
        }
        Csc {
            n: self.n,
            indptr: counts,
            indices,
            values,
        }
    }
}

/// Compressed sparse column matrix (square, f64). Row indices within each
/// column are ascending (guaranteed by [`Csr::to_csc`]).
#[derive(Clone, Debug)]
pub struct Csc {
    /// Row count == column count.
    pub n: usize,
    /// Column pointers, length `n + 1`.
    pub indptr: Vec<usize>,
    /// Row indices per stored entry.
    pub indices: Vec<usize>,
    /// Stored entry values.
    pub values: Vec<f64>,
}

/// Sparse lower-triangular Cholesky factor in CSC layout; applying it as a
/// preconditioner solves `L Lᵀ x = b` by forward + back substitution.
/// (Port of pymatting `CholeskyDecomposition`.)
pub struct IcholFactor {
    n: usize,
    lv: Vec<f64>,
    lr: Vec<usize>,
    lp: Vec<usize>,
}

impl IcholFactor {
    /// Apply the preconditioner: return `x` with `L Lᵀ x = b`.
    pub fn apply(&self, b: &[f64]) -> Vec<f64> {
        let mut x = b.to_vec();
        // Forward substitution with L (CSC columns; diagonal first).
        for j in 0..self.n {
            let k = self.lp[j];
            let temp = x[j] / self.lv[k];
            x[j] = temp;
            for k in self.lp[j] + 1..self.lp[j + 1] {
                x[self.lr[k]] -= self.lv[k] * temp;
            }
        }
        // Back substitution with Lᵀ.
        for i in (0..self.n).rev() {
            let mut s = x[i];
            for k in self.lp[i] + 1..self.lp[i + 1] {
                s -= self.lv[k] * x[self.lr[k]];
            }
            x[i] = s / self.lv[self.lp[i]];
        }
        x
    }
}

/// Inner column-oriented ichol kernel (direct port of pymatting `_ichol`).
/// Returns nnz on success, -1 for insufficient positive-definiteness at this
/// shift, -2 for max_nnz overflow.
#[allow(clippy::too_many_arguments)]
fn ichol_kernel(
    n: usize,
    av: &[f64],
    ar: &[usize],
    ap: &[usize],
    lv: &mut [f64],
    lr: &mut [usize],
    lp: &mut [usize],
    discard_threshold: f64,
    shift: f64,
    max_nnz: usize,
    relative_discard_threshold: f64,
    diag_keep_discarded: bool,
) -> i64 {
    const NONE: usize = usize::MAX;
    let mut nnz = 0usize;
    let mut c_n = 0usize;
    let mut s = vec![0usize; n]; // next non-zero row index in column j of L
    let mut t = vec![0usize; n]; // first subdiagonal index in column j of A
    let mut l = vec![NONE; n]; // linked list of non-zero columns in row k of L
    let mut a = vec![0f64; n]; // values of column j
    let mut r = vec![0f64; n]; // r[j] = sum(abs(A[j:, j]))
    let mut b = vec![false; n]; // b[i]: element i of column j non-zero
    let mut c = vec![0usize; n]; // row indices of non-zeros in column j
    let mut d = vec![shift; n]; // diagonal elements
    for j in 0..n {
        for idx in ap[j]..ap[j + 1] {
            let i = ar[idx];
            if i == j {
                d[j] += av[idx];
                t[j] = idx + 1;
            }
            if i >= j {
                r[j] += av[idx].abs();
            }
        }
    }
    for j in 0..n {
        for idx in t[j]..ap[j + 1] {
            let i = ar[idx];
            let l_ij = av[idx];
            if l_ij != 0.0 && i > j {
                a[i] += l_ij;
                if !b[i] {
                    b[i] = true;
                    c[c_n] = i;
                    c_n += 1;
                }
            }
        }
        let mut k = l[j];
        while k != NONE {
            let k0 = s[k];
            let k1 = lp[k + 1];
            let k2 = l[k];
            let l_jk = lv[k0];
            let k0 = k0 + 1;
            if k0 < k1 {
                s[k] = k0;
                let i = lr[k0];
                l[k] = l[i];
                l[i] = k;
                for idx in k0..k1 {
                    let i = lr[idx];
                    a[i] -= lv[idx] * l_jk;
                    if !b[i] {
                        b[i] = true;
                        c[c_n] = i;
                        c_n += 1;
                    }
                }
            }
            k = k2;
        }
        if d[j] <= 0.0 {
            return -1;
        }
        if nnz + 1 + c_n > max_nnz {
            return -2;
        }
        d[j] = d[j].sqrt();
        lv[nnz] = d[j];
        lr[nnz] = j;
        nnz += 1;
        s[j] = nnz;
        c[..c_n].sort_unstable();
        for &i in c[..c_n].iter() {
            let l_ij = a[i] / d[j];
            if diag_keep_discarded {
                d[i] -= l_ij * l_ij;
            }
            let rel = relative_discard_threshold * r[j];
            if l_ij.abs() > discard_threshold && a[i].abs() > rel {
                if !diag_keep_discarded {
                    d[i] -= l_ij * l_ij;
                }
                lv[nnz] = l_ij;
                lr[nnz] = i;
                nnz += 1;
            }
            a[i] = 0.0;
            b[i] = false;
        }
        c_n = 0;
        lp[j + 1] = nnz;
        if lp[j] + 1 < lp[j + 1] {
            let i = lr[lp[j] + 1];
            l[j] = l[i];
            l[i] = j;
        }
    }
    nnz as i64
}

/// Thresholded incomplete Cholesky decomposition of a symmetric positive
/// definite CSC matrix (port of pymatting `ichol`, default parameters).
///
/// Tries the pymatting shift ladder until the factorization succeeds.
pub fn ichol(a: &Csc, discard_threshold: f64) -> Result<IcholFactor, String> {
    let n = a.n;
    // Upper bound: full A pattern (lower triangle) plus diagonal; the matting
    // Laplacian pattern comfortably fits (pymatting uses a large fixed cap).
    let max_nnz = a.values.len() + n + 1;
    let mut lv = vec![0f64; max_nnz];
    let mut lr = vec![0usize; max_nnz];
    let mut lp = vec![0usize; n + 1];
    let shifts = [
        0.0, 1e-4, 1e-3, 1e-2, 0.1, 0.5, 1.0, 10.0, 100.0, 1e3, 1e4, 1e5,
    ];
    for &shift in shifts.iter() {
        let nnz = ichol_kernel(
            n,
            &a.values,
            &a.indices,
            &a.indptr,
            &mut lv,
            &mut lr,
            &mut lp,
            discard_threshold,
            shift,
            max_nnz,
            0.0,
            true,
        );
        if nnz >= 0 {
            lv.truncate(nnz as usize);
            lr.truncate(nnz as usize);
            return Ok(IcholFactor { n, lv, lr, lp });
        }
        if nnz == -2 {
            return Err("ichol: max_nnz overflow".to_string());
        }
        // -1: retry with the next shift; reset pointers.
        lp.iter_mut().for_each(|p| *p = 0);
    }
    Err("ichol: insufficient positive-definiteness at all shifts".to_string())
}

/// Preconditioned conjugate gradients (port of pymatting `cg.py`, the
/// non-reorthogonalized path). `precondition` maps residual -> z.
pub fn cg<F>(
    a: &Csr,
    b: &[f64],
    precondition: F,
    rtol: f64,
    atol: f64,
    maxiter: usize,
) -> Result<Vec<f64>, String>
where
    F: Fn(&[f64]) -> Vec<f64>,
{
    let n = b.len();
    assert_eq!(a.n, n);
    let norm = |v: &[f64]| v.iter().map(|x| x * x).sum::<f64>().sqrt();
    let inner = |u: &[f64], v: &[f64]| u.iter().zip(v).map(|(x, y)| x * y).sum::<f64>();

    let mut x = vec![0f64; n];
    let norm_b = norm(b);
    let mut ax = vec![0f64; n];
    a.matvec(&x, &mut ax);
    let mut r: Vec<f64> = b.iter().zip(&ax).map(|(bi, axi)| bi - axi).collect();
    let mut norm_r = norm(&r);
    if norm_r < atol || norm_r < rtol * norm_b {
        return Ok(x);
    }
    let z = precondition(&r);
    let mut p = z.clone();
    let mut rz = inner(&r, &z);
    let mut ap = vec![0f64; n];
    for _ in 0..maxiter {
        a.matvec(&p, &mut ap);
        let alpha = rz / inner(&p, &ap);
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        norm_r = norm(&r);
        if norm_r < atol || norm_r < rtol * norm_b {
            return Ok(x);
        }
        let z = precondition(&r);
        let mut beta = 1.0 / rz;
        rz = inner(&r, &z);
        beta *= rz;
        for i in 0..n {
            p[i] = p[i] * beta + z[i];
        }
    }
    Err(format!(
        "cg: no convergence within {maxiter} iterations (|r| = {norm_r:e})"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dense_to_csr(m: &[&[f64]]) -> Csr {
        let n = m.len();
        let mut indptr = vec![0usize];
        let mut indices = Vec::new();
        let mut values = Vec::new();
        for row in m {
            for (j, &v) in row.iter().enumerate() {
                if v != 0.0 {
                    indices.push(j);
                    values.push(v);
                }
            }
            indptr.push(indices.len());
        }
        Csr {
            n,
            indptr,
            indices,
            values,
        }
    }

    #[test]
    fn ichol_matches_pymatting_doctest() {
        // pymatting ichol docstring: A = [[2, 3], [3, 5]], apply to [1, 2]
        // yields [-1, 1] (exact factorization for a 2x2 SPD matrix).
        let a = dense_to_csr(&[&[2.0, 3.0], &[3.0, 5.0]]);
        let f = ichol(&a.to_csc(), 1e-4).unwrap();
        let x = f.apply(&[1.0, 2.0]);
        assert!((x[0] + 1.0).abs() < 1e-12, "x0 = {}", x[0]);
        assert!((x[1] - 1.0).abs() < 1e-12, "x1 = {}", x[1]);
    }

    #[test]
    fn cg_solves_small_spd_system() {
        // pymatting cg docstring system: A = [[3, 1], [1, 2]], b = [4, 3]
        // -> x = [1, 1].
        let a = dense_to_csr(&[&[3.0, 1.0], &[1.0, 2.0]]);
        let x = cg(&a, &[4.0, 3.0], |r| r.to_vec(), 1e-12, 0.0, 100).unwrap();
        assert!((x[0] - 1.0).abs() < 1e-9);
        assert!((x[1] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn ichol_preconditioned_cg_on_laplacian_like_matrix() {
        // 1-D Poisson matrix (SPD, tridiagonal), n = 50: CG + ichol must
        // reach a much tighter residual in few iterations and match the
        // analytic solution of A x = ones (a parabola).
        let n = 50;
        let mut rows: Vec<Vec<f64>> = vec![vec![0.0; n]; n];
        for i in 0..n {
            rows[i][i] = 2.0;
            if i > 0 {
                rows[i][i - 1] = -1.0;
            }
            if i + 1 < n {
                rows[i][i + 1] = -1.0;
            }
        }
        let refs: Vec<&[f64]> = rows.iter().map(|r| r.as_slice()).collect();
        let a = dense_to_csr(&refs);
        let b = vec![1.0; n];
        let f = ichol(&a.to_csc(), 1e-4).unwrap();
        let x = cg(&a, &b, |r| f.apply(r), 1e-10, 0.0, 1000).unwrap();
        // Analytic: x_i = (i+1)(n - i) / 2 for the discrete Poisson problem.
        // Index loop kept: the assertion references x[i] by index.
        #[allow(clippy::needless_range_loop)]
        for i in 0..n {
            let expected = ((i + 1) * (n - i)) as f64 / 2.0;
            assert!(
                (x[i] - expected).abs() < 1e-6,
                "x[{i}] = {} != {expected}",
                x[i]
            );
        }
    }

    #[test]
    fn submatrix_extracts_masked_block() {
        let a = dense_to_csr(&[&[1.0, 2.0, 0.0], &[0.0, 3.0, 4.0], &[5.0, 0.0, 6.0]]);
        let mask = [true, false, true];
        let s = a.submatrix(&mask, &mask);
        assert_eq!(s.n, 2);
        // Expected block [[1, 0], [5, 6]].
        let mut y = vec![0.0; 2];
        s.matvec(&[1.0, 1.0], &mut y);
        assert_eq!(y, vec![1.0, 11.0]);
    }
}
