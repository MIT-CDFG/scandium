#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors

"""Generate i18n/pseudo.json from i18n/en.json — deterministic.

Spec: ES §27 (strings/i18n); plan §2.2 row "i18n from day 1" (pseudo is
GENERATED, never hand-edited). Conventions ported from the proto's
``scripts/gen_pseudo.py`` (same authorship): every en message becomes
``[!!! Àççẽñŧẽđ ẽẋƥàñšîôñ ~~~ !!!]`` — accented (untranslated literals
stand out), expanded by ~30% (clipped layouts show), bracket-wrapped
(concatenation and unwrapped strings are obvious).

Structure-aware: templates are parsed with the SAME ICU-MessageFormat
subset the Rust CLI loader implements (``crates/appname-cli/src/i18n.rs``,
READ-ONLY reference): ``{name}`` placeholders ([A-Za-z0-9_]+) and cardinal
plural blocks ``{name, plural, one {...} other {...}}`` whose ``#`` is the
number. Placeholders, plural machinery, ``#``, and Qt file-filter glob
patterns ``(*.ext)`` survive verbatim; only literal text is accented.
Any other braced run is preserved verbatim (never accented), matching the
loader's keep-malformed-blocks-visible behavior.

Exemption (recorded in i18n/MINTED.md and the proto's PROTO-IDS.md):
``proto.lang.*`` endonyms pass through unchanged — the Language menu must
stay navigable in every locale.

Usage:  python3 i18n/tools/pseudo.py [--check]
        --check: exit 1 if i18n/pseudo.json is not byte-identical to what
                 would be generated (drift guard, called by audit.py);
                 writes nothing.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
I18N = REPO / "i18n"

PASSTHROUGH_PREFIXES = ("proto.lang.",)

_PLAIN = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
_FANCY = "àƀçđẽƒĝĥîĵķĺɱñôƥɋŕšŧûṽŵẋýžÀɃÇĐẼƑĜĤÎĴĶĹⱮÑÔƤɊŔŠŦÛṼŴẊÝŽ"
_ACCENT = {p: f for p, f in zip(_PLAIN, _FANCY)}
assert len(_PLAIN) == len(_FANCY) == 52, "accent map out of sync"

# Functional substrings that must survive verbatim: Qt file-dialog glob
# patterns like "(*.appname)" or "(*.png *.jpg)" stop matching files if
# their letters are accented.
_PROTECTED = re.compile(r"\(\*[^)]*\)")

_PLACEHOLDER_NAME = re.compile(r"^[A-Za-z0-9_]+$")


def _accent(text: str) -> str:
    out = []
    pos = 0
    for m in _PROTECTED.finditer(text):
        out.append("".join(_ACCENT.get(ch, ch) for ch in text[pos : m.start()]))
        out.append(m.group(0))
        pos = m.end()
    out.append("".join(_ACCENT.get(ch, ch) for ch in text[pos:]))
    return "".join(out)


def _matching_brace(chars: str, open_i: int) -> int | None:
    """Index of the '}' closing the '{' at open_i, or None (loader parity)."""
    depth = 0
    for i in range(open_i, len(chars)):
        if chars[i] == "{":
            depth += 1
        elif chars[i] == "}":
            depth -= 1
            if depth == 0:
                return i
    return None


def _parse_plural(inner: str):
    """"name, plural, one {..} other {..}" -> (name, [(sel, segments)...])
    or None — the same acceptance rules as the Rust loader's parse_plural
    (selectors limited to one|other; an `other` branch required)."""
    first = inner.find(",")
    if first < 0:
        return None
    head, rest = inner[:first], inner[first + 1 :]
    second = rest.find(",")
    if second < 0:
        return None
    kw, branches_src = rest[:second], rest[second + 1 :]
    if kw.strip() != "plural":
        return None
    name = head.strip()
    if not name or not _PLACEHOLDER_NAME.match(name):
        return None
    branches = []
    i = 0
    n = len(branches_src)
    while i < n:
        while i < n and branches_src[i].isspace():
            i += 1
        if i >= n:
            break
        sel_start = i
        while i < n and branches_src[i] != "{":
            i += 1
        if i >= n:
            return None  # selector without branch
        selector = branches_src[sel_start:i].strip()
        if selector not in ("one", "other"):
            return None  # unsupported selector
        end = _matching_brace(branches_src, i)
        if end is None:
            return None
        branches.append((selector, _parse_segments(branches_src[i + 1 : end])))
        i = end + 1
    if not any(sel == "other" for sel, _ in branches):
        return None  # an ICU plural requires an `other` branch
    return name, branches


def _parse_segments(template: str):
    """Segment list: ("text", s) | ("ph", name) | ("raw", s) |
    ("plural", name, [(sel, segments)...])."""
    segments = []
    i = 0
    n = len(template)
    text_start = i
    while i < n:
        if template[i] != "{":
            i += 1
            continue
        end = _matching_brace(template, i)
        if end is None:
            break  # unbalanced: rest is literal text (loader emits verbatim)
        if text_start < i:
            segments.append(("text", template[text_start:i]))
        inner = template[i + 1 : end]
        plural = _parse_plural(inner)
        if plural is not None:
            segments.append(("plural", plural[0], plural[1]))
        elif _PLACEHOLDER_NAME.match(inner):
            segments.append(("ph", inner))
        else:
            # not a placeholder, not a plural: keep the braces verbatim and
            # never accent them (the loader keeps such blocks visible)
            segments.append(("raw", "{" + inner + "}"))
        i = end + 1
        text_start = i
    if text_start < n:
        segments.append(("text", template[text_start:]))
    return segments


def _reserialize(segments) -> str:
    out = []
    for seg in segments:
        if seg[0] == "text":
            out.append(_accent(seg[1]))
        elif seg[0] == "ph":
            out.append("{" + seg[1] + "}")
        elif seg[0] == "raw":
            out.append(seg[1])
        else:  # ("plural", name, [(sel, segments)...])
            branches = " ".join(
                f"{sel} {{{_reserialize(body)}}}" for sel, body in seg[2]
            )
            out.append("{" + seg[1] + ", plural, " + branches + "}")
    return "".join(out)


def _letters(segments) -> int:
    n = 0
    for seg in segments:
        if seg[0] == "text":
            n += sum(ch.isalpha() for ch in seg[1])
        elif seg[0] == "plural":
            n += max((_letters(body) for _, body in seg[2]), default=0)
    return n


def pseudolocalize(template: str) -> str:
    segments = _parse_segments(template)
    body = _reserialize(segments)
    pad = min(12, max(2, round(_letters(segments) * 0.3)))
    return "[!!! " + body + " " + "~" * pad + " !!!]"


def generate() -> str:
    src = json.loads((I18N / "en.json").read_text(encoding="utf-8"))
    out = {
        "_meta": {
            "locale": "pseudo",
            "plural_rules": "one_other",
            "direction": "ltr",
            "generated": True,
            "generator": "i18n/tools/pseudo.py",
            "source": "i18n/en.json",
        }
    }
    for key, value in src.items():
        if key == "_meta" or not isinstance(value, str):
            continue
        if key.startswith(PASSTHROUGH_PREFIXES):
            out[key] = value
        else:
            out[key] = pseudolocalize(value)
    return json.dumps(out, ensure_ascii=False, indent=2) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(
        description="generate i18n/pseudo.json from i18n/en.json"
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify i18n/pseudo.json is up to date; write nothing",
    )
    args = parser.parse_args()

    target = I18N / "pseudo.json"
    text = generate()
    if args.check:
        current = target.read_text(encoding="utf-8") if target.exists() else ""
        if current != text:
            print("pseudo: i18n/pseudo.json is stale — rerun i18n/tools/pseudo.py")
            return 1
        print("pseudo: i18n/pseudo.json is up to date")
        return 0
    target.write_text(text, encoding="utf-8")
    n = len([k for k in json.loads(text) if k != "_meta"])
    print(f"pseudo: wrote i18n/pseudo.json ({n} entries)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
