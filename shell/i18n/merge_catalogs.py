# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# Spec: plan §2.2/§2.3 i18n rows; shell README "i18n" section. Build-time
# catalog merge: <repo>/i18n/<lang>.json (O-I18N-A11Y's canonical catalogs)
# + shell/i18n/ui.<lang>.json (the shell's SEED for its own minted ui.*
# keys, pending the O-I18N fold-in — the mint register in
# reports/O-UI-log.md stays the canonical handoff) -> staged
# <exe>/i18n/<lang>.json. Canonical keys always WIN on collision: the seed
# can never shadow an O-I18N-owned id (collisions are reported on stderr
# so the audit sees them).

import json
import sys


def main() -> int:
    if len(sys.argv) != 4:
        print("usage: merge_catalogs.py <canonical.json> <seed.json|-> <out.json>",
              file=sys.stderr)
        return 2
    canonical_path, seed_path, out_path = sys.argv[1], sys.argv[2], sys.argv[3]

    with open(canonical_path, "r", encoding="utf-8") as f:
        canonical = json.load(f)

    seed = {}
    if seed_path != "-":
        try:
            with open(seed_path, "r", encoding="utf-8") as f:
                seed = json.load(f)
        except FileNotFoundError:
            pass  # no seed for this locale: stage the canonical file as-is

    # Catalog format: flat map, or the map nested under "messages";
    # "_meta" rides at top level. Merge at the message-map level.
    def message_map(cat):
        return cat["messages"] if isinstance(cat.get("messages"), dict) else cat

    out = dict(canonical)
    out_msgs = dict(message_map(canonical))
    seed_msgs = message_map(seed)
    for key, value in seed_msgs.items():
        if key == "_meta":
            continue
        if key in out_msgs:
            print(f"merge_catalogs: seed key shadows canonical, canonical kept: {key}",
                  file=sys.stderr)
            continue
        out_msgs[key] = value
    if isinstance(canonical.get("messages"), dict):
        out["messages"] = out_msgs
    else:
        out = {**out_msgs, "_meta": canonical.get("_meta")} if "_meta" in canonical else out_msgs

    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(out, f, ensure_ascii=False, indent=2, sort_keys=True)
        f.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
