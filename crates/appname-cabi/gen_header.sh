#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Copyright (c) 2026 the Appname contributors
#
# Regenerate include/appname.h from crates/appname-cabi with cbindgen
# (build-time tool; deps/ record owned by O-BUILD). The committed header is
# drift-checked on every `cargo test` (crates/appname-cabi/tests/c_smoke.rs,
# ES Appendix N.2 `abi_sym_header_drift` discipline) — regenerate + commit
# together with the Rust change that moved it.
#
# Lives inside the crate (not scripts/) per the O-SCRIPT write fence; the
# gate reaches it through the workspace test suite, not by path.

set -eu
cd "$(dirname "$0")/../.."

cbindgen --config crates/appname-cabi/cbindgen.toml \
         --crate appname-cabi \
         --output include/appname.h \
         crates/appname-cabi
echo "wrote include/appname.h"
