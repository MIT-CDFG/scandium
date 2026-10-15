<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- Copyright (c) 2026 the Appname contributors -->

# appname (Python) — the trusted tier

Apache-2.0 Python SDK for Appname: the ES §25.1 **trusted tier**, built
strictly **over the stable C ABI** (`appname-cabi`; the same symbols
`include/appname.h` declares — no other engine surface is touched). The
trusted tier imports one module, `appname` (ES §25.4).

## Trusted by design — read this first

This tier is **trusted by design** (ES §25.1). A Python script running
here has the full power of the process: Python itself hands it the
filesystem, the network, and `ctypes`. **There is no sandbox at this tier
and none is claimed.** Run scripts you trust, the way you would run any
program. The sandboxed story for untrusted community plugins is the
WASM/Lua tier (ES §25.3, CP-1.0), not this module.

## One door

Every document mutation is an `appname.command.v1` envelope through
`Document.apply()` — the same single entry the CLI and the shell use
(D-48). There are deliberately no convenience mutators that bypass it.

```python
import json, uuid
import appname

doc = appname.Document.new(320, 200, "white")
wire = json.loads(doc.apply(json.dumps({
    "schema": "appname.command.v1",
    "id": str(uuid.uuid4()),
    "kind": "layer.add",
    "params": {"kind": "raster", "name": "Ink", "newLayerId": str(uuid.uuid4())},
})))
assert wire["ok"], wire
doc.export("out.png", "png")
```

## Surface (CP-0.7)

- Lifecycle: `Document.new/open/save/close`, `appname.version()`
- Mutation: `apply(envelope_json)` — returns receipt-or-error wire JSON
  (`{"ok":true,"receipt":…}` / `{"ok":false,…}`), **never raises for a
  command rejection**; `undo()` / `redo()` (engine functions, ES §10.3)
- Reads: `info_json()`, `history_json()`, `poll_events()`,
  `read_region(x,y,w,h)`, `read_selection(x,y,w,h)`,
  `composite_rgba8()` (all copies at this checkpoint; §25.4's zero-copy
  memoryview is listed growth)
- Export: `export(path, format, options_json=None)` — the ABI export
  door (ABI 1.2 block)
- Assets: `asset_create(bytes)` (content-addressed id)
- Actions (§26): `ActionRecorder(name)` — `apply(doc, env)` records the
  command stream this script emits (successful applies only; rule-3
  name/index capture rides along), `add_asset(doc, bytes)`,
  `add_stop(message, allow_continue=True)`, `finish()` →
  `appname.action.v1` JSON; `Document.apply_action(action_json,
  options_json=None)` replays through the ABI's action door (ABI 1.3
  block — gate on `appname.version()[1] >= 3`); a step failure raises
  `EngineError` whose payload embeds the partial report
- Recovery (§7.8): `autosave_begin/end`, `appname.autosave_query(path)`,
  `appname.open_recovered(package, log)`, `appname.autosave_discard(path)`

Everything that raises raises `appname.EngineError`; `e.args[0]` is the
structured error JSON (`{code, message, field?}`).

Growth to the full §25.4 surface (`Application`, `LayerProxy`,
`Selection`, `register_filter/action`, `undo_group`, zero-copy reads,
`appname.ui`) lands with its checkpoints.

## Build & test

- Wheel: `maturin build --release -m crates/appname-py/Cargo.toml`
  (abi3, CPython 3.10+).
- Tests: `pytest crates/appname-py/tests -q` (builds the module with
  plain cargo; no maturin needed).

Licensing: this crate's code is Apache-2.0 (DD ④ SDK split). The built
extension links the GPL-3.0-only engine crates through `appname-cabi`
(recorded DD-area-4 posture), so a distributed wheel is GPL-governed as
a combined work.
