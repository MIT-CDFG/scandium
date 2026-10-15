<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# Software Bill of Materials — scope and coverage

This directory carries the CycloneDX SBOM for Appname, one `*.cdx.json` document per workspace
crate, generated at package time by `cargo-cyclonedx` from the Cargo dependency graph.

## What this SBOM covers

**The Rust dependency graph only.** Every component listed here is a Cargo crate that Appname's own
Rust code depends on, resolved from `Cargo.lock`.

As measured on 2026-09-12: 21 documents, 2,005 component entries, 265 distinct crates.

## What this SBOM does NOT cover, and why

**It does not describe the third-party libraries this application ships.** The macOS bundle carries
its Qt frameworks and their transitive dependencies, collected by `macdeployqt` at package time.
Those arrive through the linker's graph, not through Cargo's, so `cargo-cyclonedx` cannot see them
and does not report them. Searching this SBOM for any of them returns nothing:

    qt · glib · gobject · gio · openssl · brotli · pcre · dbus · graphite2 · gettext · xz
        -> absent from every document here

⚠ **A name in this SBOM that matches a shipped library is usually not that library.** The Rust
binding crates are named after the C libraries they bind, and they are pinned to their own versions,
which are not the versions of the binaries in this bundle. For example this SBOM lists `harfbuzz-sys`
0.8.0, `lcms2-sys` 4.0.7 and `freetype-sys` 0.23.0 — those are Rust crates. The corresponding shipped
libraries are separate binaries at their own, different versions. **A licence or a version read from a
crate entry here is not evidence about the binary in `Contents/Frameworks`.**

## The population of record

For the question *"what third-party binaries does this artifact actually ship, and under what
licence"*, the authority is **not this SBOM**. It is the shipped-library enumeration walked directly
off the built bundle — every Mach-O file in the application, with its project, version, licence and
dependency record — together with `THIRD-PARTY-NOTICES.md` and the licence texts in
`Contents/Resources/licenses/`.

**This SBOM is a Rust-graph document and must not be read as artifact coverage.** It is published
because it is accurate about what it describes, and this note exists so that it is not mistaken for
something larger.
