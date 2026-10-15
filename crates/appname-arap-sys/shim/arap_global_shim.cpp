// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// The ARAP global-step shim TU (deps/libigl.yaml + the O-INT 2026-08-17
// consumption ruling: file-level MPL via a cpp shim from the PINNED
// EXTERNAL checkout — libigl headers are consumed UNMODIFIED from
// outside the tree; nothing is vendored). Include closure: exactly
// <igl/min_quad_with_fixed.h> (a member of the record's 127-file ARAP
// closure inventory) + Eigen core/sparse under the deps/eigen.yaml
// sanctioned shape (-DEIGEN_MPL2_ONLY defined by build.rs; no
// unsupported/, no BLAS/LAPACK backends). ZERO references into
// include/igl/copyleft/ — guarded by build.rs and the in-tree
// copyleft-exclusion test.
//
// Contract (see src/puppet/arap_libigl.rs): factor the free-vertex SPD
// Laplacian once per mesh + pin topology (§18.4 factor-once), then
// back-substitute per-iteration RHS vectors. min_quad_with_fixed
// minimizes ½·xᵀQx + xᵀB (no fixed rows here — hard-constraint
// elimination is Rust-side), so Q = L and B = −b give L·x = b.

#include <igl/min_quad_with_fixed.h>

#include <Eigen/Core>
#include <Eigen/Sparse>

#include <vector>

namespace {

struct MqwfHandle {
    igl::min_quad_with_fixed_data<double> data;
    int n = 0;
};

} // namespace

extern "C" {

void* appname_arap_mqwf_new(int n, const double* triplets, int n_triplets) {
    if (n <= 0 || (n_triplets > 0 && triplets == nullptr)) {
        return nullptr;
    }
    std::vector<Eigen::Triplet<double>> t;
    t.reserve(static_cast<size_t>(n_triplets));
    for (int k = 0; k < n_triplets; ++k) {
        const int i = static_cast<int>(triplets[3 * k + 0]);
        const int j = static_cast<int>(triplets[3 * k + 1]);
        if (i < 0 || j < 0 || i >= n || j >= n) {
            return nullptr;
        }
        t.emplace_back(i, j, triplets[3 * k + 2]);
    }
    Eigen::SparseMatrix<double> q(n, n);
    q.setFromTriplets(t.begin(), t.end());

    auto* h = new (std::nothrow) MqwfHandle();
    if (h == nullptr) {
        return nullptr;
    }
    h->n = n;
    const Eigen::VectorXi known; // empty: elimination happens Rust-side
    const Eigen::SparseMatrix<double> aeq(0, n);
    // pd = true: the reduced Laplacian is SPD by construction.
    if (!igl::min_quad_with_fixed_precompute(q, known, aeq, true, h->data)) {
        delete h;
        return nullptr;
    }
    return h;
}

int appname_arap_mqwf_solve(void* handle, const double* b, double* out) {
    auto* h = static_cast<MqwfHandle*>(handle);
    if (h == nullptr || b == nullptr || out == nullptr) {
        return 1;
    }
    // ½·xᵀQx + xᵀB with B = −b ⇒ Q·x = b.
    Eigen::VectorXd bvec = -Eigen::Map<const Eigen::VectorXd>(b, h->n);
    const Eigen::VectorXd y;   // no known values
    const Eigen::VectorXd beq; // no equality constraints
    Eigen::VectorXd z;
    if (!igl::min_quad_with_fixed_solve(h->data, bvec, y, beq, z)) {
        return 2;
    }
    if (z.size() != h->n) {
        return 3;
    }
    Eigen::Map<Eigen::VectorXd>(out, h->n) = z;
    return 0;
}

void appname_arap_mqwf_free(void* handle) {
    delete static_cast<MqwfHandle*>(handle);
}

} // extern "C"
