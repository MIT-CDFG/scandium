// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

// macOS extension modules resolve CPython symbols at import time; this adds
// the `-undefined dynamic_lookup` linker args so plain `cargo build` links
// (maturin adds them by itself, but the workspace must build without it —
// proto precedent).
fn main() {
    pyo3_build_config::add_extension_module_link_args();
}
