#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# SBOM generation — plan §2.4 Packaging v1 (SBOM row); CP-1.0 Appendix O
# attestation groundwork. Tool record: deps/cargo-cyclonedx.yaml.
#
# Output: sbom/*.cdx.json (CycloneDX per workspace crate). The sbom/
# directory is GITIGNORED by decision: CycloneDX embeds a random
# serialNumber + timestamp, so output is not byte-reproducible and a
# committed copy would drift every run. Regenerate on demand.
#
# WHO ATTACHES IT: build_dmg.sh stage 4c runs this script and stages
# sbom/*.cdx.json, plus packaging/macos/sbom-README.md, into the bundle at
# Contents/Resources/sbom/ before codesign. Until that stage existed this
# header's claim that "release packaging attaches the fresh SBOM" was false --
# nothing attached it and no .cdx.json reached any artifact (E-05).
#
# WHAT IT COVERS: the CARGO graph, and nothing else. Qt and everything
# macdeployqt collects into Contents/Frameworks are outside cargo-cyclonedx's
# reach and absent from this output. The population of record for the shipped
# artifact is the shipped-library enumerator walked off the staged bundle.
# sbom-README.md ships that boundary alongside the documents; do not let this
# output be read as artifact coverage.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! command -v cargo >/dev/null 2>&1; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi
if ! cargo cyclonedx --version >/dev/null 2>&1; then
    echo "gen_sbom: cargo-cyclonedx not installed (see deps/cargo-cyclonedx.yaml; cargo install cargo-cyclonedx --locked)" >&2
    exit 1
fi

# cargo-cyclonedx 0.5.x writes <crate>.cdx.json beside each Cargo.toml
# (no output-dir flag); generate, then collect EVERY output into gitignored
# sbom/ — workspace members live under crates/, tests/, and the root, and
# stray generated files must never pollute other lanes' trees.
# OB-77. This directory was append-only: `mkdir -p` and `mv` in, never a removal.
# So a document survives the crate it describes -- a rename, a removal, or (as
# happened on 2026-09-14 13:39) a 62-byte TEST FIXTURE I planted here by hand
# stays in the population, gets staged into the bundle by stage 4c and copied to
# the release page by stage 9. It reached neither only because the certificate
# was cut in a fresh worktree. The SBOM must describe THIS tree, not every tree
# this directory has ever seen.
rm -rf sbom
mkdir -p sbom
cargo cyclonedx --format json --describe crate
find . -name '*.cdx.json' -not -path './sbom/*' -not -path './target/*' -exec mv {} sbom/ \;
# OPERATOR WORD 9 (8): the built cabi/py artifacts are GPL-3.0-only combined works.
# This does NOT rewrite the emitted documents -- WORD 8 (2) ruled that the source of
# truth is what gets fixed, and both manifests now carry GPL-3.0-only. What belongs
# here is the ASSERTION that the generator actually produced it, because a licence
# fixed in a manifest and silently un-emitted is the same defect one layer down.
for _c in appname-cabi appname-py; do
    _doc="sbom/${_c}.cdx.json"
    [ -f "$_doc" ] || { echo "gen_sbom: $_doc missing — cannot assert the combined-work licence" >&2; exit 1; }
    _lic="$(python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print(((d.get('metadata',{}).get('component',{}).get('licenses') or [{}])[0]).get('expression',''))" "$_doc")"
    if [ "$_lic" != "GPL-3.0-only" ]; then
        echo "gen_sbom: $_doc declares metadata.component licence '${_lic:-<none>}', expected GPL-3.0-only — the built artifact is a combined work (WORD 9 (8)); fix crates/${_c}/Cargo.toml, not this document" >&2
        exit 1
    fi
done
echo "gen_sbom: combined-work assertion OK (appname-cabi, appname-py -> GPL-3.0-only)"

# OB-79. crates/appname-py/Cargo.toml says `version.workspace = true`, so the SBOM's
# component version follows Cargo.toml; crates/appname-py/pyproject.toml hardcodes
# its own and derives from nothing. Today both read 0.1.0 -- by coincidence, not by
# construction. At E-11's freeze bump the workspace becomes 1.0.2, the SBOM follows,
# and the wheel keeps declaring 0.1.0 while this document says otherwise. An SBOM
# that disagrees with the artifact it describes is the defect, so the check lives
# here rather than in a ledger entry.
_py_sbom="$(python3 -c "import json,sys; print(json.load(open('sbom/appname-py.cdx.json'))['metadata']['component'].get('version',''))")"
_py_proj="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\(.*\)"/\1/p' crates/appname-py/pyproject.toml | head -1)"
if [ "$_py_sbom" != "$_py_proj" ]; then
    echo "gen_sbom: appname-py version disagrees — SBOM says '${_py_sbom:-<none>}' (from Cargo.toml, which appname-py inherits) but crates/appname-py/pyproject.toml says '${_py_proj:-<none>}'. The wheel and the document describe different versions; fix pyproject.toml (OB-79, E-11)." >&2
    exit 1
fi
echo "gen_sbom: appname-py version agreement OK (Cargo/SBOM and pyproject both $_py_sbom)"

ls -1 sbom | wc -l | xargs echo "gen_sbom: wrote component files:"
