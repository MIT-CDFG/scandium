<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# Scandium

**Website:** <https://mit-cdfg.github.io/scandium/> — downloads, first-run steps, what is in 1.0.

Scandium is a free, open-source image editor for macOS (Apple silicon) and Windows (x64). It
works in layers, with masks and adjustment layers, painting and retouching tools, and selections,
and it opens and exports PNG, JPEG, TIFF, WebP, GIF and BMP. Scandium is GPL-3.0-only; its SDK
interface is Apache-2.0. It makes no network connections, collects no telemetry and needs no
account.

## Download

Get the current release from the
[Releases page](https://github.com/MIT-CDFG/scandium/releases): the macOS disk image (`.dmg`) and
the Windows zip. Each release carries a `SHA256SUMS` file beside the downloads; check what you
downloaded before opening it:

```
shasum -a 256 -c SHA256SUMS
```

**Opening it the first time on macOS.** The disk image is signed ad-hoc and is not notarized, so
macOS refuses `Scandium.app` on a plain double-click. Control-click (or right-click) the app,
choose **Open**, then **Open** again in the dialog. macOS remembers the choice, and later launches
are normal. Scandium 1.0.2 needs macOS 15.4 or later on Apple silicon. Alternatively, open it
once, then go to **System Settings → Privacy & Security** and choose **Open Anyway**.

**Opening it the first time on Windows.** The executable is unsigned, so SmartScreen shows a
"Windows protected your PC" notice on the first run. Choose **More info**, then **Run anyway**.
Scandium needs the Microsoft Visual C++ Redistributable (x64); install it from Microsoft first if
it is not already on the machine.

## Known issues

- [`KNOWN-ISSUES.md`](KNOWN-ISSUES.md) lists what is known not to work yet, with workarounds where
  they exist.
- [`RELEASE-NOTES-1.0.md`](RELEASE-NOTES-1.0.md) describes what is in this release and what is not.
- [`USER-GUIDE.md`](USER-GUIDE.md) is the guide to the tools and menus.

## Building from source

Scandium is a Rust engine behind a stable C ABI, driven headlessly or by a Qt/QML shell, with
every document mutation flowing through one deterministic `appname.command.v1` command entry.
`contracts/` is a generated verbatim mirror of the frozen behavior spec (schemas, command catalog,
algorithm and patent registers, message ids) — never edit it by hand; it is refreshed and verified
by `tools/sync_contracts.sh`. External dependencies require a record in `deps/` before they land
and must pass `cargo deny`. `scripts/gate.sh` is the single local gate entry.

On macOS, with Rust, Qt 6 and Ninja installed:

```
cargo build --workspace
cmake -S shell -B target/shell-build -G Ninja \
      -DCMAKE_PREFIX_PATH=/opt/homebrew/opt/qt -DCMAKE_BUILD_TYPE=Debug
cmake --build target/shell-build
```

[`BUILDING.md`](BUILDING.md) covers release builds on macOS and Windows from a clean checkout,
and [`shell/README.md`](shell/README.md) covers the shell itself.

## License

The engine and UI are GPL-3.0-only (see [`LICENSE`](LICENSE)). The C ABI crate
(`appname-cabi`), the generated headers and the SDK surface are Apache-2.0 (see
[`LICENSE-SDK`](LICENSE-SDK)). Third-party components and their licenses are listed in
[`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md).
