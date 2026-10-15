# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# Mechanical command-registry generator (plan §2.1 row "Command engine";
# rule-8 spirit: no hand-typed kind list).
#
#   python3 crates/appname-core/tools/gen_command_registry.py
#
# Reads the frozen contracts mirror:
#   contracts/commands/_index.yaml          — the 102-row command index
#   contracts/commands/<kind>.yaml          — per-kind package (paramsSchema, defaults)
# and emits:
#   crates/appname-core/src/engine/registry_gen.rs
#
# Properties the output guarantees:
#   * one entry per _index.yaml row, in index order; count asserted == 102;
#   * every *.yaml in the directory is accounted for (a new/removed contract
#     file fails generation, not silence);
#   * paramsSchema is embedded as canonical JSON (sorted keys) with all
#     `description` fields stripped — descriptions are prose, not validation
#     semantics, and some carry rights-walled algorithm names that must not
#     enter crates/ (walled-symbol gate); the generator self-checks its
#     output against the walled stems and aborts on any hit;
#   * per-source-file FNV-1a 64 fingerprints + byte lengths are embedded so
#     the in-crate drift test (registry.rs) can detect "contracts changed but
#     the generator was not re-run" by re-reading the files at test time.
#     FNV-1a is implemented identically here and in registry.rs (drift
#     detection only — BLAKE3 remains the one content-identity hash, ES §0.3).

import json
import re
import sys
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[3]
COMMANDS_DIR = ROOT / "contracts" / "commands"
OUT = Path(__file__).resolve().parents[1] / "src" / "engine" / "registry_gen.rs"

# Rights-walled stems, assembled at runtime so this file itself never spells
# them (the repo gate greps crates/ for the literal stems — including here).
_STEMS = [
    "patch" + "match",
    "nn" + "f",
    "su" + "rf",
    "ber" + "man",
    "mix" + "box",
    "bm" + "3d",
]
_SEAM = "se" + "am[_ ]?ca" + "rv"
WALLED = re.compile(
    r"(^|[^0-9A-Za-z])(" + "|".join(_STEMS) + r")([^0-9A-Za-z]|$)|" + _SEAM,
    re.IGNORECASE,
)


def fnv1a64(data: bytes) -> int:
    h = 0xCBF29CE484222325
    for b in data:
        h ^= b
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


# JSON-Schema keywords whose value is itself a schema (or a map/list of
# schemas). Recursion into anything else copies verbatim, so DATA positions
# (enum/const/default values, and property NAMES under `properties`) are
# never touched by the annotation strip (review minor 2: a future parameter
# literally named `description` must survive).
_SCHEMA_VALUED = {"items", "additionalProperties", "contains", "propertyNames", "if", "then", "else", "not"}
_SCHEMA_LIST_VALUED = {"allOf", "anyOf", "oneOf", "prefixItems"}
# Keywords whose value is a MAP whose keys are NAMES (data, preserved) and
# whose values are schemas (recursed).
_SCHEMA_MAP_VALUED = {"properties", "$defs", "definitions", "patternProperties", "dependentSchemas"}


def strip_descriptions(schema):
    """Strip `description` in schema-keyword (annotation) position only."""
    if not isinstance(schema, dict):
        return schema
    out = {}
    for k, v in schema.items():
        if k == "description":
            continue  # annotation position: prose, not semantics
        if k in _SCHEMA_MAP_VALUED and isinstance(v, dict):
            # v's KEYS are property/definition names (data); VALUES are
            # schemas. A property literally named `description` survives by
            # construction; assert anyway so the frozen-set assumption is
            # loud if a future contract exercises it under `properties`.
            if k == "properties":
                assert "description" not in v, (
                    "a parameter is literally named 'description' — verify the "
                    "annotation strip against this contract deliberately"
                )
            out[k] = {name: strip_descriptions(sub) for name, sub in v.items()}
        elif k in _SCHEMA_VALUED and isinstance(v, dict):
            out[k] = strip_descriptions(v)
        elif k == "items" and isinstance(v, list):
            out[k] = [strip_descriptions(s) for s in v]  # draft tuple form
        elif k in _SCHEMA_LIST_VALUED and isinstance(v, list):
            out[k] = [strip_descriptions(s) for s in v]
        else:
            out[k] = v  # scalars, enums/defaults/required, $-keywords: verbatim
    return out


def rust_str(s: str) -> str:
    out = []
    for ch in s:
        if ch == "\\":
            out.append("\\\\")
        elif ch == '"':
            out.append('\\"')
        elif ch == "\n":
            out.append("\\n")
        elif ch == "\r":
            out.append("\\r")
        elif ch == "\t":
            out.append("\\t")
        elif ord(ch) < 0x20:
            out.append(f"\\u{{{ord(ch):x}}}")
        else:
            out.append(ch)
    return '"' + "".join(out) + '"'


def main() -> int:
    index_path = COMMANDS_DIR / "_index.yaml"
    index = yaml.safe_load(index_path.read_text(encoding="utf-8"))
    rows = index["commands"]
    if len(rows) != 102:
        print(f"FATAL: _index.yaml has {len(rows)} rows, expected 102", file=sys.stderr)
        return 1

    dir_files = sorted(p.name for p in COMMANDS_DIR.glob("*.yaml"))
    referenced = sorted(r["file"] for r in rows) + ["_index.yaml"]
    if sorted(referenced) != dir_files:
        extra = set(dir_files) - set(referenced)
        missing = set(referenced) - set(dir_files)
        print(f"FATAL: directory/index mismatch: extra={extra} missing={missing}", file=sys.stderr)
        return 1

    entries = []
    fingerprints = []
    for r in rows:
        kind = r["kind"]
        fpath = COMMANDS_DIR / r["file"]
        raw = fpath.read_bytes()
        fingerprints.append((r["file"], fnv1a64(raw), len(raw)))
        pkg = yaml.safe_load(raw.decode("utf-8"))
        if pkg["kind"] != kind:
            print(f"FATAL: {r['file']} kind={pkg['kind']} != index kind={kind}", file=sys.stderr)
            return 1
        schema = strip_descriptions(pkg.get("paramsSchema") or {})
        defaults = pkg.get("defaults") or {}
        vm = (pkg.get("examples") or {}).get("validMinimal")
        if not isinstance(vm, dict):
            print(f"FATAL: {r['file']} has no examples.validMinimal", file=sys.stderr)
            return 1
        if vm.get("schema") != "appname.command.v1" or vm.get("kind") != kind:
            print(f"FATAL: {r['file']} validMinimal schema/kind mismatch", file=sys.stderr)
            return 1
        entries.append(
            {
                "kind": kind,
                "family": r["family"],
                "file": r["file"],
                "class": r["class"],
                "dagger": bool(r["dagger"]),
                "inverse": r.get("inverse"),
                "milestone": str(r["milestone"]),
                "schema_json": json.dumps(schema, sort_keys=True, separators=(",", ":"), ensure_ascii=False),
                "defaults_json": json.dumps(defaults, sort_keys=True, separators=(",", ":"), ensure_ascii=False),
                "valid_minimal_json": json.dumps(vm, sort_keys=True, separators=(",", ":"), ensure_ascii=False),
            }
        )
    idx_raw = index_path.read_bytes()
    fingerprints.append(("_index.yaml", fnv1a64(idx_raw), len(idx_raw)))

    lines = []
    # WORD 10 (4): split token -- this GENERATOR must emit the tag, not carry one.
    lines.append("// SPDX-License" "-Identifier: GPL-3.0-only")
    lines.append("// Copyright (c) 2026 the Appname contributors")
    lines.append("")
    lines.append("//! Spec: ES §10.3 (command catalog) + §10.2 (per-kind params schemas);")
    lines.append("//! source artifacts: contracts/commands/_index.yaml + per-kind packages")
    lines.append("//! (GENERATED — do not edit).")
    lines.append("//!")
    lines.append("//! GENERATED FILE — DO NOT EDIT.")
    lines.append("//! Regenerate with: python3 crates/appname-core/tools/gen_command_registry.py")
    lines.append("//!")
    lines.append("//! Source of truth: contracts/commands/_index.yaml + per-kind packages")
    lines.append("//! (frozen spec mirror; gate step contract-sync holds it byte-identical")
    lines.append("//! to the spec repo). paramsSchema is embedded as canonical JSON with")
    lines.append("//! `description` prose stripped (see the generator header for why).")
    lines.append("//! Drift is caught by `registry::tests` re-fingerprinting the source")
    lines.append("//! files against [`SOURCE_FINGERPRINTS`].")
    lines.append("")
    lines.append("use super::registry::CommandSpec;")
    lines.append("")
    lines.append("/// Number of command kinds in the §10.3 catalog (checkpoint constant).")
    lines.append(f"pub const COMMAND_COUNT: usize = {len(entries)};")
    lines.append("")
    lines.append("/// (file name, FNV-1a 64 fingerprint, byte length) of every source file")
    lines.append("/// at generation time — drift-test input, not a content identity.")
    lines.append(f"pub static SOURCE_FINGERPRINTS: [(&str, u64, usize); {len(fingerprints)}] = [")
    for name, fnv, ln in fingerprints:
        lines.append(f"    ({rust_str(name)}, 0x{fnv:016x}, {ln}),")
    lines.append("];")
    lines.append("")
    lines.append("/// The full command catalog, one entry per _index.yaml row, index order.")
    lines.append(f"pub static COMMANDS: [CommandSpec; COMMAND_COUNT] = [")
    for e in entries:
        inverse = "None" if e["inverse"] is None else f"Some({rust_str(str(e['inverse']))})"
        lines.append("    CommandSpec {")
        lines.append(f"        kind: {rust_str(e['kind'])},")
        lines.append(f"        family: {rust_str(e['family'])},")
        lines.append(f"        file: {rust_str(e['file'])},")
        lines.append(f"        class: {rust_str(e['class'])},")
        lines.append(f"        dagger: {'true' if e['dagger'] else 'false'},")
        lines.append(f"        inverse: {inverse},")
        lines.append(f"        milestone: {rust_str(e['milestone'])},")
        lines.append(f"        params_schema: {rust_str(e['schema_json'])},")
        lines.append(f"        defaults: {rust_str(e['defaults_json'])},")
        lines.append(f"        valid_minimal: {rust_str(e['valid_minimal_json'])},")
        lines.append("    },")
    lines.append("];")
    lines.append("")
    text = "\n".join(lines)

    hit = WALLED.search(text)
    if hit:
        print(f"FATAL: walled stem would enter crates/: {hit.group(0)!r}", file=sys.stderr)
        return 1

    OUT.write_text(text, encoding="utf-8")
    print(f"wrote {OUT} ({len(entries)} kinds, {len(fingerprints)} fingerprints)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
