# SPDX-License-Identifier: Apache-2.0
# Copyright (c) 2026 the Appname contributors

"""appname trusted-tier round-trip suite (ES 25.1 over the C ABI).

Every mutation goes through Document.apply() with an appname.command.v1
envelope — the single entry (D-48). Covers: lifecycle, apply wire parity,
undo/redo, readback (region/selection/composite), export door, action
record + invoke (ABI 1.3), asset carriage, autosave/recovery surface.
"""

import json
import uuid

import pytest

import appname


def env(kind, params=None, target=None, eid=None):
    e = {"schema": "appname.command.v1", "id": eid or str(uuid.uuid4()), "kind": kind}
    if params is not None:
        e["params"] = params
    if target is not None:
        e["target"] = target
    return json.dumps(e)


def ok(doc_or_rec, *args):
    """apply() through a Document or ActionRecorder; assert acceptance."""
    r = json.loads(doc_or_rec.apply(*args))
    assert r["ok"], f"rejected: {r}"
    return r["receipt"]


def rejected(doc, envelope):
    r = json.loads(doc.apply(envelope))
    assert not r["ok"], f"unexpectedly accepted: {r}"
    return r


def info(doc):
    return json.loads(doc.info_json())


def layer_id(doc, name):
    for row in info(doc)["layers"]:
        if row["name"] == name:
            return row["id"]
    raise AssertionError(f"no layer named {name}")


def add_layer(doc, name):
    ok(
        doc,
        env("layer.add", {"kind": "raster", "name": name, "newLayerId": str(uuid.uuid4())}),
    )
    return layer_id(doc, name)


def test_version_gates_the_13_block():
    v = appname.version()
    assert v[0] == 1
    assert v[1] >= 3, "the action-invoke door needs abi_minor >= 3"


def test_lifecycle_apply_wire_parity_and_history():
    doc = appname.Document.new(80, 60, "white")
    rows = info(doc)
    assert (rows["width"], rows["height"]) == (80, 60)
    assert rows["layers"][0]["name"] == "Background"

    # Acceptance: receipt wire shape.
    receipt = ok(doc, env("layer.add", {"kind": "raster", "name": "Ink", "newLayerId": str(uuid.uuid4())}))
    assert receipt["kind"] == "layer.add"

    # Rejection: NO exception, CLI-parity wire document (25.4 apply
    # semantics), taxonomy code intact.
    r = rejected(doc, env("no.such.kind"))
    assert r["error"]["code"] == "error.command.unknown_kind"
    dup = env("layer.add", {"kind": "raster", "name": "Dup", "newLayerId": str(uuid.uuid4())},
              eid="0a0a0a0a-1111-4000-8000-000000000001")
    ok(doc, dup)
    r = rejected(doc, dup)
    assert r["error"]["code"] == "error.schema.duplicate_id"

    # Undo/redo are engine functions returning history rows.
    row = json.loads(doc.undo())
    assert row["kind"] == "layer.add"
    row = json.loads(doc.redo())
    assert row["kind"] == "layer.add"
    hist = json.loads(doc.history_json())
    assert hist["length"] == 3  # doc.new + 2 adds
    assert hist["cursor"] == 3

    # Events drain (9.5 shapes) and then report empty.
    events = json.loads(doc.poll_events())
    assert isinstance(events, list) and events
    assert json.loads(doc.poll_events()) == []

    # Explicit close makes reuse a TYPED EngineError.
    doc.close()
    with pytest.raises(appname.EngineError) as e:
        doc.info_json()
    assert "error.api.invalid_arg" in e.value.args[0]


def test_readback_region_selection_composite():
    doc = appname.Document.new(16, 8, "white")
    data, w, h = doc.read_region(0, 0, 16, 8)
    assert (w, h) == (16, 8)
    assert len(data) == 16 * 8 * 4
    assert data[:4] == b"\xff\xff\xff\xff", "white background reads white, straight alpha"
    cdata, cw, ch = doc.composite_rgba8()
    assert (cw, ch) == (16, 8) and cdata == data

    # Selection coverage: zero before, 255 inside a rect select after.
    sdata, sw, sh = doc.read_selection(0, 0, 16, 8)
    assert set(sdata) == {0}
    ok(doc, env("selection.set", {"source": "rect", "rect": {"x": 2, "y": 1, "w": 4, "h": 3}}))
    sdata, sw, sh = doc.read_selection(0, 0, 16, 8)
    assert sdata[1 * 16 + 2] == 255 and sdata[0] == 0


def test_export_door_and_package_round_trip(tmp_path):
    doc = appname.Document.new(12, 10, "white")
    out = tmp_path / "out.png"
    doc.export(str(out), "png")
    assert out.read_bytes()[:4] == b"\x89PNG"

    with pytest.raises(appname.EngineError) as e:
        doc.export(str(tmp_path / "x.tga"), "tga")
    assert "error.codec.unrecognized" in e.value.args[0]

    pkg = tmp_path / "doc.appname"
    add_layer(doc, "Kept")
    doc.save(str(pkg))
    reopened = appname.Document.open(str(pkg))
    names = [r["name"] for r in info(reopened)["layers"]]
    assert "Kept" in names
    # The reopened document accepts commands through the same door.
    add_layer(reopened, "After")


def test_action_record_then_invoke():
    rec_doc = appname.Document.new(32, 24, "white")
    target = add_layer(rec_doc, "Retouch")

    rec = appname.ActionRecorder("Tint Retouch")
    ok(rec, rec_doc, env("layer.set_props", {"opacity": 0.5}, {"layerId": target}))
    rec.add_stop("check the tint", True)
    # A REJECTED apply is not recorded (rule-2 discipline).
    r = json.loads(rec.apply(rec_doc, env("no.such.kind")))
    assert not r["ok"]
    assert len(rec) == 2

    action = json.loads(rec.finish())
    assert action["schema"] == "appname.action.v1"
    assert action["steps"][0]["recordedTarget"]["layerName"] == "Retouch"

    # Invoke on a DIFFERENT document: same-named layer, different id —
    # rule-3 name resolution through the ABI door.
    play_doc = appname.Document.new(32, 24, "white")
    add_layer(play_doc, "Retouch")
    report = json.loads(play_doc.apply_action(json.dumps(action)))
    assert report["status"] == "ok"
    assert [s["status"] for s in report["steps"]] == ["ok", "stop"]
    assert report["steps"][1]["message"] == "check the tint"
    row = next(r for r in info(play_doc)["layers"] if r["name"] == "Retouch")
    assert abs(row["opacity"] - 0.5) < 1e-6


def test_action_step_failure_raises_with_embedded_report():
    doc = appname.Document.new(8, 8, "white")
    action = {
        "schema": "appname.action.v1",
        "name": "grain",
        "steps": [
            {
                "command": {
                    "kind": "layer.set_props",
                    "target": {"layerId": str(uuid.uuid4())},
                    "params": {"opacity": 0.4},
                },
                "recordedTarget": {"layerName": "Grain", "treeIndex": 30},
            }
        ],
    }
    with pytest.raises(appname.EngineError) as e:
        doc.apply_action(json.dumps(action))
    payload = json.loads(e.value.args[0])
    assert payload["messageId"] == "error.batch.step_failed"
    assert "Grain" in payload["message"]
    assert payload["report"]["steps"][0]["status"] == "error"


def test_action_vars_and_asset_carriage(tmp_path):
    # An asset recorded on one document replays onto another (content-
    # addressed carriage) with a parameterized paste position.
    src = appname.Document.new(6, 6, "white")
    png = tmp_path / "blob.png"
    src.export(str(png), "png")
    blob = png.read_bytes()

    rec_doc = appname.Document.new(24, 24, "white")
    rec = appname.ActionRecorder("paste blob")
    asset_id = rec.add_asset(rec_doc, blob)
    ok(
        rec,
        rec_doc,
        env(
            "edit.paste",
            {"assetId": asset_id, "at": [2, 2], "newLayerId": str(uuid.uuid4())},
        ),
    )
    action = json.loads(rec.finish())
    assert action["assets"][0]["id"] == asset_id
    action["steps"][0]["parameterized"] = ["/params/at"]

    fresh = appname.Document.new(24, 24, "white")
    report = json.loads(
        fresh.apply_action(
            json.dumps(action), json.dumps({"vars": {"/params/at": [5, 7]}})
        )
    )
    assert report["status"] == "ok"
    names = [r["name"] for r in info(fresh)["layers"]]
    assert "Pasted" in names

    # Unknown options key is a typed reject.
    with pytest.raises(appname.EngineError) as e:
        fresh.apply_action(json.dumps(action), json.dumps({"nope": 1}))
    assert "error.api.invalid_arg" in e.value.args[0]


def test_autosave_recovery_surface(tmp_path):
    pkg = tmp_path / "base.appname"
    log = tmp_path / "doc.autosave"
    doc = appname.Document.new(20, 12, "white")
    doc.save(str(pkg))
    doc.autosave_begin(str(log))
    add_layer(doc, "Unsaved")
    # Simulated crash: drop the handle without saving.
    doc.close()

    q = json.loads(appname.autosave_query(str(log)))
    assert q["commands"] == 1 and not q["torn"]

    recovered, report = appname.open_recovered(str(pkg), str(log))
    rep = json.loads(report)
    assert rep["replayed"] == 1
    names = [r["name"] for r in info(recovered)["layers"]]
    assert "Unsaved" in names

    appname.autosave_discard(str(log))
    assert not log.exists()
    # Idempotent.
    appname.autosave_discard(str(log))


def test_trusted_tier_docs_say_so():
    """ES 25.1: the tier is trusted BY DESIGN and its docs must say so."""
    docs = appname.Document.__doc__ or ""
    readme = (
        __import__("pathlib").Path(__file__).resolve().parents[1] / "README.md"
    ).read_text()
    assert "trusted" in readme.lower()
    assert "sandbox" in readme.lower()


def test_version_gates_the_14_block():
    v = appname.version()
    assert v[0] == 1
    assert v[1] >= 4, "the per-layer readback door needs abi_minor >= 4"


def test_read_layer_content_not_contribution():
    doc = appname.Document.new(16, 8, "white")
    bg = info(doc)["layers"][0]["id"]
    ink = add_layer(doc, "Ink")
    ok(
        doc,
        env(
            "fill",
            {"content": "color", "color": {"model": "rgb", "comps": [1.0, 0.0, 0.0, 0.0]}},
            target={"layerId": ink},
        ),
    )
    data, w, h = doc.read_layer(ink, 0, 0, 16, 8)
    assert (w, h) == (16, 8)
    assert data[:4] == b"\xff\x00\x00\xff", "the fill is the layer's content"
    bg_data, _, _ = doc.read_layer(bg, 0, 0, 16, 8)
    assert bg_data[:4] == b"\xff\xff\xff\xff", "background content untouched"

    # The lane election: the layer's own compositing props are NOT applied.
    ok(doc, env("layer.set_props", {"opacity": 0.5}, target={"layerId": ink}))
    data2, _, _ = doc.read_layer(ink, 0, 0, 16, 8)
    assert data2 == data, "content readback ignores layer opacity"

    # Typed refusal: unknown layer id raises with the taxonomy code.
    with pytest.raises(appname.EngineError) as e:
        doc.read_layer(str(uuid.uuid4()), 0, 0, 4, 4)
    assert "error.command.bad_target" in e.value.args[0]


def test_read_layer_group_arm_serves_as_if_isolated():
    # The return-leg growth under the same symbol: a GROUP serves its
    # children composited as-if-isolated (the header's stated election) —
    # the group's own props never applied; root == whole-doc readback.
    doc = appname.Document.new(16, 8, "white")
    ink = add_layer(doc, "Ink")
    ok(
        doc,
        env(
            "fill",
            {"content": "color", "color": {"model": "rgb", "comps": [1.0, 0.0, 0.0, 0.0]}},
            target={"layerId": ink},
        ),
    )
    gid = str(uuid.uuid4())
    ok(doc, env("layer.group", {"layerIds": [ink], "newGroupId": gid, "name": "G"}))
    data, w, h = doc.read_layer(gid, 0, 0, 16, 8)
    assert (w, h) == (16, 8)
    assert data[:4] == b"\xff\x00\x00\xff", "the group serves its child's content"

    # Contribution props never applied: a hidden group still serves.
    ok(doc, env("layer.set_props", {"visible": False}, target={"layerId": gid}))
    data2, _, _ = doc.read_layer(gid, 0, 0, 16, 8)
    assert data2 == data, "hidden group serves content unchanged"

    # Root-as-group == the whole-document readback (identity contract).
    root = info(doc)["layers"][0]["parentId"]
    root_data, _, _ = doc.read_layer(root, 0, 0, 16, 8)
    region, _, _ = doc.read_region(0, 0, 16, 8)
    assert root_data == region, "root group content IS the document composite"


def test_info_v3_paths_array_present():
    doc = appname.Document.new(8, 8, "white")
    i = info(doc)
    assert i["paths"] == [], "info v3: the paths array is always present"
