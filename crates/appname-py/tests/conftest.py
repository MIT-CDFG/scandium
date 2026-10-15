# SPDX-License-Identifier: Apache-2.0
# Copyright (c) 2026 the Appname contributors

"""Import plumbing for the appname pytest suite.

Builds the extension cdylib with plain cargo (no maturin needed for
tests — proto precedent inverted: the wheel path is maturin's, the test
path is cargo's) and stages it as an importable `appname` module.
"""

import os
import pathlib
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[3]


def _cargo() -> str:
    found = shutil.which("cargo")
    if found:
        return found
    candidate = pathlib.Path.home() / ".cargo/bin/cargo"
    if candidate.exists():
        return str(candidate)
    raise RuntimeError("cargo not found for building appname-py")


def pytest_configure(config):
    subprocess.run(
        [_cargo(), "build", "-p", "appname-py"],
        cwd=ROOT,
        check=True,
    )
    if sys.platform == "darwin":
        built = ROOT / "target/debug/libappname_py.dylib"
    elif os.name == "nt":
        built = ROOT / "target/debug/appname_py.dll"
    else:
        built = ROOT / "target/debug/libappname_py.so"
    stage = ROOT / "target/pytest-appname"
    stage.mkdir(parents=True, exist_ok=True)
    dest = stage / "appname.so"
    shutil.copy2(built, dest)
    sys.path.insert(0, str(stage))
