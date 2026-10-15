<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# Building Scandium from source

This guide is for someone building the project for the first time. It covers macOS
(Apple silicon) and Windows (x64), the packaging scripts, where the third-party
notices come from, and how translations are found at run time.

Two names appear throughout and both are correct: the product is **Scandium**, while
the internal project, the crates and the executable are still called `appname`. The
executable is `appname-shell` on both platforms; only the artifacts and user-visible
text carry the product name.

## Contents

- [What you need](#what-you-need)
- [Building on macOS](#building-on-macos)
- [Building on Windows](#building-on-windows)
- [Running the gate](#running-the-gate)
- [Packaging on macOS](#packaging-on-macos)
- [Packaging on Windows](#packaging-on-windows)
- [Third-party notices](#third-party-notices)
- [Translations at run time](#translations-at-run-time)
- [Reproducible builds](#reproducible-builds)

## What you need

| | version | notes |
|---|---|---|
| Rust | **1.97.1** | pinned in `rust-toolchain.toml`; `rustup` reads it automatically |
| Qt | **6.11.1** | the minor version must match across platforms |
| CMake | **3.22** or newer | the minimum the shell's `CMakeLists.txt` declares |
| Ninja | any recent | the shell builds with the Ninja generator |
| Python | 3, with `PyYAML` | only for the notice and attestation tools; no minimum is pinned |

The Qt version is not advisory. The macOS and Windows artifacts are built from one
tag and are expected to carry the same Qt minor; check yours with `qmake --version`
before you start.

## Building on macOS

Apple silicon, with Homebrew.

```bash
brew install qt cmake ninja
rustup toolchain install 1.97.1
```

Homebrew installs Qt under `/opt/homebrew`. Confirm it is the expected version:

```bash
qmake --version          # expect: Using Qt version 6.11.1
```

Build the Rust workspace, then the Qt/QML shell:

```bash
cargo build --workspace
cmake -S shell -B target/shell-build -G Ninja -DCMAKE_BUILD_TYPE=Release -DAPPNAME_BUNDLE=ON
cmake --build target/shell-build --parallel
```

`-DAPPNAME_BUNDLE=ON` makes the shell build a macOS `.app` bundle rather than a bare
executable. Leave it off if you only want to run the shell from the build directory.

## Building on Windows

Open an **x64 Native Tools Command Prompt for VS 2022**, so that the `cc` and `cmake`
crates find MSVC. You need the VS 2022 Build Tools, Qt 6.11.1 for **MSVC 2022 64-bit**,
the Rust MSVC target, CMake and Ninja.

```
cargo build --release --workspace
cmake -S shell -B build-win -G Ninja -DCMAKE_BUILD_TYPE=Release ^
      -DCMAKE_PREFIX_PATH="C:\Qt\6.11.1\msvc2022_64" ^
      -DAPPNAME_CABI_DIR=%CD%\target\release
cmake --build build-win
windeployqt --release --qmldir shell\qml build-win\appname-shell.exe
```

`windeployqt` is not a convenience step. It copies Qt in as separate, replaceable
DLLs, which is what the project's LGPL-3.0 election requires: Qt must ship as
replaceable dynamic libraries and must never be statically absorbed into the
executable or merged by a packer. The macOS side satisfies the same duty with
`macdeployqt`.

To check that Qt really is being imported rather than absorbed:

```
dumpbin /dependents build-win\appname-shell.exe
```

`Qt6Core.dll`, `Qt6Gui.dll` and `Qt6Quick.dll` should appear as imports, and those
DLLs should be present as files beside the executable.

## Running the gate

`scripts/gate.sh` is the single local gate entry. It runs fourteen steps and prints a
PASS/FAIL line for each:

`build`, `test`, `deny`, `reuse`, `spdx`, `contract-sync`, `notices`, `i18n-audit`,
`walled-symbols`, `fmt`, `shell-relink`, `quarantine`, `release-engine-bind`,
`shell-behavior`.

```bash
bash scripts/gate.sh
```

The first four are the ones you will meet most often, and you can run them directly:

```bash
cargo build --workspace
cargo test --workspace
cargo deny check advisories bans licenses sources
cargo fmt --all --check
```

A step that cannot be measured on your machine — because a tool is not installed, for
instance — reports UNMEASURED rather than passing. That is deliberate: a check that
did not run is not a check that succeeded.

## Packaging on macOS

```bash
bash scripts/build_dmg.sh
```

The script runs in ten numbered stages: a licence and notice pre-flight, the release
binaries, icon provenance, the shell build, bundle staging, the Qt frameworks,
`codesign`, the disk image, the notarization boundary, and the release set. It stops
at the first failure with a line naming the cause, and a cold run takes a few hours.

It produces, under `target/`:

- `Scandium-<version>-macOS-arm64.dmg` — the disk image, volume name `Scandium <version>`
- `release-set/` — the image, the source tarball, the per-crate CycloneDX SBOMs, and
  `SHA256SUMS` over everything in the directory
- `dmg-logs/` — the per-stage logs and the release attestation

The artifact name is derived from `packaging/PRODUCT_NAME` and the workspace version
in `Cargo.toml`, not written into the script.

Two environment variables change what it does:

- `APPNAME_SIGN_IDENTITY` — sign with a real identity and the hardened runtime instead
  of an ad-hoc signature. Unset, the build is signed ad-hoc, which is what the 1.0
  release ships; see the Gatekeeper note in `README.md`.
- `APPNAME_EXTRA_ARTIFACTS` — a directory whose files are copied into the release set
  before `SHA256SUMS` is written, so an artifact built on another machine (the Windows
  zip) can be covered by the same manifest and attestation.

## Packaging on Windows

The zip contains one top-level folder and never explodes into the current directory:

```
Scandium-<version>-Windows-x64.zip
└── Scandium\
    ├── appname-shell.exe
    ├── appname_cabi.dll
    ├── Qt6*.dll, platforms\, styles\, imageformats\, qml\   (windeployqt output)
    ├── i18n\                  <- next to the exe; see below
    ├── LICENSE                <- GPL-3.0-only
    ├── LICENSE-SDK            <- Apache-2.0
    ├── THIRD-PARTY-NOTICES.md
    ├── LICENSES\              <- SPDX texts, and it must contain LGPL-3.0-only.txt
    └── README.md
```

`LICENSES\LGPL-3.0-only.txt` is required, because Qt ships under that election. The
repository's own `LICENSES/` directory does **not** contain it: that directory holds
only the licences this repository's own files declare, and the Qt text is carriage
shipped with an artifact. Copy it from:

```
packaging/macos/notices/LGPL-3.0-only.txt
```

The path says `macos` only because macOS carried it first; the bytes are
platform-neutral and are the same text the macOS bundle ships.

```
powershell Compress-Archive -Path <stage>\Scandium -DestinationPath Scandium-<version>-Windows-x64.zip
```

## Third-party notices

`THIRD-PARTY-NOTICES.md` is generated, not written by hand:

```bash
python3 tools/gen_notices.py            # regenerate
python3 tools/gen_notices.py --check    # verify the committed file matches
```

It is built from `Cargo.lock`, the dependency records in `deps/*.yaml`, and vendor
provenance records; licence texts are read from the exact locked package versions and
deduplicated by hash. The gate's `notices` step runs `--check`, so a stale committed
file fails the gate.

The generator takes a `--platform` argument, because one tag ships two artifacts and
each carries its own copy of the file. The Rust dependency closure is identical on
both platforms — it is taken over all targets — while the native libraries bundled
beside the binary differ. `--platform macos` is the default. `--platform windows`
currently refuses, rather than emitting a file: the Windows native set is decided by
`windeployqt` and is not yet recorded, and notices that silently omit a bundled
library's licence would be worse than none.

Adding a dependency requires a record in `deps/` before it lands, and it must pass
`cargo deny`.

## Translations at run time

Catalogues are resolved in this order:

1. `APPNAME_I18N_DIR`, if set
2. an `i18n/` directory beside the executable
3. the embedded English catalogue

On Windows, `i18n\` must be copied next to `appname-shell.exe`. Anywhere else and the
application silently falls back to English — which passes a smoke test and fails a
translated user. The macOS packaging stops the build if the bundle's `i18n/en.json`
is missing.

## Reproducible builds

`tools/repro-build.sh` normalises the environment for a byte-comparable build:

```bash
eval "$(bash tools/repro-build.sh --print-env-sh)"
cargo build --release --workspace
```

The packaging script sources those same settings before it builds the release
binaries, so a packaged build and a normalised local build share their environment.
The project's own check is that two normalised CLI builds of the freeze tree are
byte-identical; `tools/repro/allowlist.toml` records the paths exempt from that
comparison.

Note that the generated SBOM files are *not* byte-reproducible: `cargo-cyclonedx`
stamps a wall-clock timestamp into each document, so two runs from an identical
`Cargo.lock` differ in exactly that field.
