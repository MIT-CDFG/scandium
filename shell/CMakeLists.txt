# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# Spec: plan §2.2 "Qt/QML shell v1" row. Build mechanism (documented in
# shell/README.md): a cargo custom target builds crates/appname-cabi, and
# the shell links the produced library directly by absolute path — the
# corrosion integration is O-BUILD's later refinement (flagged in the lane
# log). The shell is a pure C-ABI client: include/appname.h is its ONLY
# view of the engine.

cmake_minimum_required(VERSION 3.22)
# OPERATOR WORD 9 (3), 2026-09-14 18:46 ET, and AMENDMENT 10 (e): `Cargo.toml
# [workspace.package] version` is the SINGLE SOURCE OF TRUTH for every
# version-bearing file, and this file's bundle versions read from it. They did
# not: `0.3.0` here and `0.5.0`/`0.5` below, against a workspace at `0.1.0` --
# four strings for one product, and the SHIPPED plist was measured carrying
# `CFBundleShortVersionString 0.5` on 2026-09-14 (R-11, OB-65). E-11 bumps the
# workspace to 1.0.2 at freeze; without this the artifact keeps saying 0.5.
# O-UI owns this file; their consent is given by WORD 9 (3).
#
# Exactly ONE line of that manifest begins with `version` (measured 2026-09-14),
# so any other count is a shape change and STOPS. No literal fallback: a default
# is precisely how three strings drifted from one source.
set(APPNAME_CARGO_TOML "${CMAKE_CURRENT_SOURCE_DIR}/../Cargo.toml")
if(NOT EXISTS "${APPNAME_CARGO_TOML}")
    message(FATAL_ERROR "APPNAME: ${APPNAME_CARGO_TOML} not found — cannot derive the bundle version")
endif()
file(STRINGS "${APPNAME_CARGO_TOML}" _appname_version_lines REGEX "^version[ \t]*=")
list(LENGTH _appname_version_lines _appname_version_n)
if(NOT _appname_version_n EQUAL 1)
    message(FATAL_ERROR "APPNAME: expected exactly ONE top-level `version =` line in Cargo.toml, found ${_appname_version_n} — refusing to guess which is [workspace.package]'s")
endif()
list(GET _appname_version_lines 0 _appname_version_line)
if(NOT _appname_version_line MATCHES "\"([^\"]+)\"")
    message(FATAL_ERROR "APPNAME: could not parse a quoted version out of: ${_appname_version_line}")
endif()
set(APPNAME_VERSION "${CMAKE_MATCH_1}")

# OD-1's product name, single source: packaging/PRODUCT_NAME (OPERATOR WORD 10).
set(APPNAME_PRODUCT_NAME_FILE "${CMAKE_CURRENT_SOURCE_DIR}/../packaging/PRODUCT_NAME")
if(NOT EXISTS "${APPNAME_PRODUCT_NAME_FILE}")
    message(FATAL_ERROR "APPNAME: ${APPNAME_PRODUCT_NAME_FILE} not found — the bundle name has no source")
endif()
file(STRINGS "${APPNAME_PRODUCT_NAME_FILE}" _appname_product_lines)
list(GET _appname_product_lines 0 APPNAME_PRODUCT_NAME)
string(STRIP "${APPNAME_PRODUCT_NAME}" APPNAME_PRODUCT_NAME)
if(APPNAME_PRODUCT_NAME STREQUAL "")
    message(FATAL_ERROR "APPNAME: packaging/PRODUCT_NAME is empty")
endif()

project(appname-shell VERSION ${APPNAME_VERSION} LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(Qt6 6.5 REQUIRED COMPONENTS Core Gui Qml Quick QuickControls2 Svg Test)
qt_standard_project_setup(REQUIRES 6.5)

# ---- appname-cabi via cargo ------------------------------------------------
# The repo root is shell/.. (works from any worktree of the build repo).
get_filename_component(APPNAME_REPO_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/.." ABSOLUTE)

if(CMAKE_BUILD_TYPE STREQUAL "Release")
    set(APPNAME_CARGO_PROFILE "release")
    set(APPNAME_CARGO_FLAG "--release")
else()
    set(APPNAME_CARGO_PROFILE "debug")
    set(APPNAME_CARGO_FLAG "")
endif()

set(APPNAME_CABI_DIR "${APPNAME_REPO_ROOT}/target/${APPNAME_CARGO_PROFILE}")

option(APPNAME_CABI_BUILD "Run cargo to (re)build appname-cabi before linking" ON)

# shared (default): link libappname_cabi.dylib. The cargo-built dylib
#   carries an absolute install name (dev-machine builds); packaging/rpath
#   policy is O-BUILD's packaging wave.
# static: link libappname_cabi.a — self-contained binary.
set(APPNAME_CABI_LINK "shared" CACHE STRING "How to link appname-cabi: shared|static")

if(APPNAME_CABI_LINK STREQUAL "static")
    set(APPNAME_CABI_LIB "${APPNAME_CABI_DIR}/libappname_cabi.a")
else()
    set(APPNAME_CABI_LIB "${APPNAME_CABI_DIR}/libappname_cabi${CMAKE_SHARED_LIBRARY_SUFFIX}")
endif()

# W-06 (O-WIN): Rust on the MSVC target names its outputs without the `lib`
# prefix — appname_cabi.dll with the import library appname_cabi.dll.lib, and
# the staticlib appname_cabi.lib. Shared links the import library; the DLL is
# copied beside the binaries in this build dir (the exe and the flat test
# binaries), since Windows has no rpath and resolves DLLs exe-adjacent.
if(WIN32)
    set(APPNAME_CABI_DLL "${APPNAME_CABI_DIR}/appname_cabi.dll")
    if(APPNAME_CABI_LINK STREQUAL "static")
        set(APPNAME_CABI_LIB "${APPNAME_CABI_DIR}/appname_cabi.lib")
    else()
        set(APPNAME_CABI_LIB "${APPNAME_CABI_DIR}/appname_cabi.dll.lib")
    endif()
endif()

if(APPNAME_CABI_BUILD AND WIN32)
    # W-06 (O-WIN): no PATH prefix here — the `$ENV{HOME}/.cargo/bin:` form
    # below joins with ':', which on Windows fuses into the first PATH entry
    # and drops MSVC's link.exe for Git-Bash's coreutils `link`. cargo is on
    # PATH from rustup's installer.
    add_custom_target(appname_cabi_build
        COMMAND cargo build -p appname-cabi ${APPNAME_CARGO_FLAG}
        COMMAND ${CMAKE_COMMAND} -E copy_if_different
                "${APPNAME_CABI_DLL}" "${CMAKE_CURRENT_BINARY_DIR}"
        WORKING_DIRECTORY "${APPNAME_REPO_ROOT}"
        BYPRODUCTS "${APPNAME_CABI_LIB}" "${CMAKE_CURRENT_BINARY_DIR}/appname_cabi.dll"
        COMMENT "cargo build -p appname-cabi (${APPNAME_CARGO_PROFILE})"
        VERBATIM)
elseif(APPNAME_CABI_BUILD)
    # BYPRODUCTS teaches Ninja which file the cargo run produces (cargo
    # itself is the freshness authority; a fresh tree is a fast no-op).
    add_custom_target(appname_cabi_build
        COMMAND ${CMAKE_COMMAND} -E env "PATH=$ENV{HOME}/.cargo/bin:$ENV{PATH}"
                cargo build -p appname-cabi ${APPNAME_CARGO_FLAG}
        WORKING_DIRECTORY "${APPNAME_REPO_ROOT}"
        BYPRODUCTS "${APPNAME_CABI_LIB}"
        COMMENT "cargo build -p appname-cabi (${APPNAME_CARGO_PROFILE})"
        VERBATIM)
endif()

# ---- the shell application -------------------------------------------------
qt_add_executable(appname-shell
    src/main.cpp
    src/abi_client.cpp
    src/abi_client.h
    src/announcer.cpp
    src/announcer.h
    src/canvas_view.cpp
    src/canvas_view.h
    src/clipboard_ops.cpp
    src/clipboard_ops.h
    src/document_controller.cpp
    src/document_controller.h
    src/envelope_builder.cpp
    src/envelope_builder.h
    src/font_meta.cpp
    src/font_meta.h
    src/icon_provider.cpp
    src/icon_provider.h
    src/i18n_catalog.cpp
    src/i18n_catalog.h
    src/license_texts.cpp
    src/license_texts.h
    src/layer_thumb_provider.cpp
    src/layer_thumb_provider.h
    src/sample_blob.cpp
    src/sample_blob.h
    src/selection_scan.cpp
    src/selection_scan.h
    src/tool_key_dispatcher.cpp
    src/tool_key_dispatcher.h
    src/tour.cpp
    src/tour.h
    src/units.cpp
    src/units.h
)

set_source_files_properties(qml/Theme.qml PROPERTIES QT_QML_SINGLETON_TYPE TRUE)

# Vendored Lucide icons (deps/lucide.yaml; VERBATIM files, render-time
# recoloring via LucideIconProvider) + the co-located LICENSE.
file(GLOB APPNAME_LUCIDE_SVGS CONFIGURE_DEPENDS "icons/lucide/*.svg")
qt_add_resources(appname-shell "lucide_icons"
    PREFIX "/icons/lucide"
    BASE "icons/lucide"
    FILES ${APPNAME_LUCIDE_SVGS} icons/lucide/LICENSE)

qt_add_qml_module(appname-shell
    URI AppnameShell
    VERSION 1.0
    QML_FILES
        qml/Theme.qml
        qml/Main.qml
        qml/CanvasContextMenu.qml
        qml/ChordMenuItem.qml
        qml/ToolPalette.qml
        qml/LayersPanel.qml
        qml/HistoryPanel.qml
        qml/MaskPanel.qml
        qml/NewDocumentDialog.qml
        qml/dialogs/AdjustmentParamDialog.qml
        qml/dialogs/CurveEditor.qml
        qml/dialogs/AssignProfileDialog.qml
        qml/dialogs/RecoveryDialog.qml
        qml/dialogs/CanvasSizeDialog.qml
        qml/dialogs/ConvertProfileDialog.qml
        qml/dialogs/CropDialog.qml
        qml/dialogs/DepthConvertDialog.qml
        qml/dialogs/FillDialog.qml
        qml/dialogs/FlattenConfirmDialog.qml
        qml/dialogs/DeleteConfirmDialog.qml
        qml/dialogs/AboutDialog.qml
        qml/dialogs/LicensesDialog.qml
        qml/dialogs/DiscardConfirmDialog.qml
        qml/dialogs/ResizeDialog.qml
        qml/dialogs/RotateDialog.qml
        qml/dialogs/ModeConvertDialog.qml
        qml/dialogs/FilterParamDialog.qml
        qml/dialogs/SelectionModifyDialog.qml
        qml/dialogs/TrimDialog.qml
)

# Default: plain executable (CLI-style tour + dev runs). -DAPPNAME_BUNDLE=ON
# builds the same target as a .app with the D-43 PLACEHOLDER identity
# (real name/identifier wait on OD-1; dmg/signing/notarization stay
# O-BUILD's packaging wave — this is the bundle layout only). The
# exe-adjacent i18n staging lands in Contents/MacOS/i18n inside the
# bundle, which the documented resolution order already finds.
option(APPNAME_BUNDLE "Build the shell as a macOS .app bundle (placeholder identity)" OFF)
if(APPNAME_BUNDLE)
    set_target_properties(appname-shell PROPERTIES
        MACOSX_BUNDLE TRUE
        # E-02 / OPERATOR WORD 9 (2026-09-14 20:01 ET): "CFBundleName and
        # CFBundleDisplayName 'Scandium'". CMake has NO MACOSX_BUNDLE_DISPLAY_NAME
        # property, and Qt's Info.plist.app.in defines fourteen keys, none of them
        # CFBundleDisplayName -- measured on the 6.11.1 template and confirmed on a
        # built artifact (PlistBuddy: Entry ":CFBundleDisplayName" Does Not Exist).
        # So the key CANNOT EXIST without a template of our own. That is why
        # fourteen consecutive O-WATCH reviews could report it ABSENT while every
        # rename sweep came back clean: it is an ABSENCE, and a sweep for the old
        # product name enumerates presences.
        # packaging/Info.plist.in is authored here rather than forked from Qt, adds
        # exactly one key to Qt's set (key-set diff in the lane log), and reads
        # CFBundleName and CFBundleDisplayName from the SAME variable so the two can
        # never drift -- the ninth-literal defect the comment below names.
        # ⛔ KEPT AWAY FROM THE MACOSX_BUNDLE_GUI_IDENTIFIER LINE ON PURPOSE (C-941, OB-86).
        # This block first sat directly above it, and E-02's identifier commit rewrites
        # that line, so every merge of the two conflicted on one hunk -- and of its three
        # resolutions one dropped THIS property silently: no template, no
        # CFBundleDisplayName, and build_dmg.sh reads that key zero times, so the dmg built
        # GREEN without it. With unchanged lines between the two edits, git merges them
        # cleanly and nobody is ever asked to choose. Do not move it back beside the id.
        MACOSX_BUNDLE_INFO_PLIST "${CMAKE_CURRENT_SOURCE_DIR}/packaging/Info.plist.in"
        # OPERATOR WORD 9 (4) + WORD 10: ONE product name, read from the same file
        # build_dmg.sh reads, so the CMake bundle name is not a ninth literal that
        # gets missed when the name moves.
        MACOSX_BUNDLE_BUNDLE_NAME "${APPNAME_PRODUCT_NAME}"
        MACOSX_BUNDLE_GUI_IDENTIFIER "org.scandium-editor.Scandium"
        MACOSX_BUNDLE_BUNDLE_VERSION "${APPNAME_VERSION}"
        MACOSX_BUNDLE_SHORT_VERSION_STRING "${APPNAME_VERSION}"
        # OB-55: Qt's Info.plist.app.in substitutes MACOSX_BUNDLE_COPYRIGHT into
        # NSHumanReadableCopyright. Unset, the shipped bundle carries no copyright
        # line at all — measured on the artifact 2026-09-13.
        MACOSX_BUNDLE_COPYRIGHT "Copyright (c) 2026 the Appname contributors"
        # ⛔ THE FILENAME IS A KNOWN MISNOMER SINCE 2026-09-17 (E-02). That file
        # now holds the SCANDIUM PRODUCT ICON, not the D-43 placeholder: the
        # generator's default mode emits it and the checked-in .icns was
        # regenerated from that default in the same commit, which keeps
        # build_dmg.sh's :278 and :321 cmps green because both sides moved
        # together. The NAME cannot move here alone -- build_dmg.sh hard-codes it
        # at :261 and cmps the bundle's copy of that exact name at :321, so this
        # property and that line land in ONE commit with O-BUILD, never apart.
        # Recorded in packaging/appname-placeholder.icns.PROVENANCE.md too.
        MACOSX_BUNDLE_ICON_FILE "appname-placeholder.icns")
    # The committed .icns is the checked-in output of
    # packaging/make_placeholder_icon.py + `iconutil -c icns` (regenerate
    # with those two commands; generation inside the ninja graph is
    # blocked by the sandboxed environment on this machine — recorded in
    # the lane log for O-BUILD's packaging wave).
    set(APPNAME_ICNS "${CMAKE_CURRENT_SOURCE_DIR}/packaging/appname-placeholder.icns")
    target_sources(appname-shell PRIVATE "${APPNAME_ICNS}")
    set_source_files_properties("${APPNAME_ICNS}" PROPERTIES
        MACOSX_PACKAGE_LOCATION "Resources")
else()
    set_target_properties(appname-shell PROPERTIES MACOSX_BUNDLE FALSE)
endif()

# ---- i18n catalog staging ---------------------------------------------------
# O-I18N-A11Y's catalogs live at <repo>/i18n/*.json (merged to main ahead of
# this shell precisely so it consumes them). Stage them executable-adjacent
# at build time — the documented resolution order (APPNAME_I18N_DIR ->
# exe-adjacent i18n/ -> key-echo) is unchanged; this only makes the default
# windowed run and tour find the real catalogs, exactly like an installed
# layout would.
#
# The staged en.json is MERGED with the shell's seed catalog for its own
# minted ui.* keys (shell/i18n/ui.en.json — see merge_catalogs.py header;
# canonical keys always win, collisions reported). The mint register in
# reports/O-UI-log.md remains the canonical O-I18N fold-in handoff; ids
# with no canonical/seed entry still key-echo — the standing honesty
# mechanism. Locales without a seed stage canonical-as-is (es/ja/pseudo
# key-echo the ui.* ids until the fold-in).
file(GLOB APPNAME_I18N_CATALOGS CONFIGURE_DEPENDS "${APPNAME_REPO_ROOT}/i18n/*.json")
find_package(Python3 COMPONENTS Interpreter REQUIRED)
if(APPNAME_I18N_CATALOGS)
    add_custom_command(TARGET appname-shell POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E make_directory "$<TARGET_FILE_DIR:appname-shell>/i18n"
        COMMAND ${CMAKE_COMMAND} -E copy_if_different
                ${APPNAME_I18N_CATALOGS} "$<TARGET_FILE_DIR:appname-shell>/i18n"
        COMMAND ${Python3_EXECUTABLE} "${CMAKE_CURRENT_SOURCE_DIR}/i18n/merge_catalogs.py"
                "${APPNAME_REPO_ROOT}/i18n/en.json"
                "${CMAKE_CURRENT_SOURCE_DIR}/i18n/ui.en.json"
                "$<TARGET_FILE_DIR:appname-shell>/i18n/en.json"
        COMMENT "Staging i18n catalogs exe-adjacent (repo i18n/*.json + shell ui.* seed merge)"
        VERBATIM)

    # ---- CONTENT-TRACKED RE-STAGING (2026-08-29) ----
    #
    # THE BUG THIS FIXES, CAUGHT BY A TEST WRITTEN FOR SOMETHING ELSE.
    # The POST_BUILD above runs only when appname-shell RELINKS. Editing
    # i18n/en.json changes nothing ninja tracks as an input to that
    # target, so `ninja appname-shell` after a catalog edit is a NO-OP,
    # the staged copy stays whatever it was, and every Main.qml-harness
    # test binary keeps reading the PREVIOUS strings.
    #
    # ★★★ IT IS SILENT IN BOTH DIRECTIONS AND WORSE IN ONE. A desk that
    # edits a string and sees the test still assert the OLD value reads it
    # as "my edit did not take" and goes looking in the wrong place -- that
    # is the loud half. The quiet half: a test that PINS the old value
    # stays GREEN across the edit, so a catalog change can land, be
    # believed, and be contradicted by the shipped binary with nothing red
    # anywhere. The glob is CONFIGURE_DEPENDS, which re-runs cmake when the
    # SET of files changes and notices nothing when their CONTENTS do --
    # a completeness check standing in for a freshness one.
    #
    # The stamp makes the staging depend on the catalogs THEMSELVES and on
    # the merge script, and every target that reads a staged catalog
    # depends on the stamp. Bundle-OFF (the default) stages straight into
    # the build dir, which IS the exe dir, so the POST_BUILD above becomes
    # a copy_if_different of identical bytes rather than the only writer.
    # C-706: THE COMMAND BELOW READS THREE INPUTS AND THIS LIST NAMED TWO.
    # `shell/i18n/ui.en.json` — the shell's own minted-key seed — was READ by
    # merge_catalogs.py and absent from DEPENDS, so editing the seed changed
    # nothing ninja tracked: the stamp stayed fresh, no re-stage ran, and the
    # staged catalog kept its canonical-only contents. That is this block's
    # OWN bug one input over, and its own comment above states the cost:
    # "a catalog change can land, be believed, and be contradicted by the
    # shipped binary with nothing red anywhere." For a LICENSE ATTRIBUTION
    # string (B-09) that is a compliance defect under a green battery.
    #
    # NOT an oversight by the author: the seed did not exist until the first
    # one was minted (C-700). A DEPENDS list is a claim about a SET, and
    # adding a member to that set falsifies it with nothing re-checking.
    #
    # Listed conditionally because the seed is OPTIONAL by design —
    # merge_catalogs.py handles FileNotFoundError and stages canonical as-is,
    # and an unconditional DEPENDS on a missing file is a hard ninja error.
    # CAVEAT, stated rather than discovered later: creating the seed when it
    # did not exist at configure time needs a cmake re-run to be tracked.
    set(APPNAME_UI_SEED "${CMAKE_CURRENT_SOURCE_DIR}/i18n/ui.en.json")
    set(APPNAME_I18N_STAMP_DEPS ${APPNAME_I18N_CATALOGS}
        "${CMAKE_CURRENT_SOURCE_DIR}/i18n/merge_catalogs.py")
    if(EXISTS "${APPNAME_UI_SEED}")
        list(APPEND APPNAME_I18N_STAMP_DEPS "${APPNAME_UI_SEED}")
    endif()
    set(APPNAME_I18N_STAMP "${CMAKE_CURRENT_BINARY_DIR}/i18n-staged.stamp")
    add_custom_command(
        OUTPUT "${APPNAME_I18N_STAMP}"
        COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_CURRENT_BINARY_DIR}/i18n"
        COMMAND ${CMAKE_COMMAND} -E copy_if_different
                ${APPNAME_I18N_CATALOGS} "${CMAKE_CURRENT_BINARY_DIR}/i18n"
        COMMAND ${Python3_EXECUTABLE} "${CMAKE_CURRENT_SOURCE_DIR}/i18n/merge_catalogs.py"
                "${APPNAME_REPO_ROOT}/i18n/en.json"
                "${CMAKE_CURRENT_SOURCE_DIR}/i18n/ui.en.json"
                "${CMAKE_CURRENT_BINARY_DIR}/i18n/en.json"
        COMMAND ${CMAKE_COMMAND} -E touch "${APPNAME_I18N_STAMP}"
        DEPENDS ${APPNAME_I18N_STAMP_DEPS}
        COMMENT "Re-staging i18n catalogs (contents changed)"
        VERBATIM)
    add_custom_target(appname_i18n_stage DEPENDS "${APPNAME_I18N_STAMP}")
    add_dependencies(appname-shell appname_i18n_stage)
    # Bundle-ON hygiene (perf-run finding): under -DAPPNAME_BUNDLE=ON the
    # exe-adjacent staging above lands INSIDE the .app (Contents/MacOS/
    # i18n) while the test binaries stay flat in the build dir with no
    # i18n/ neighbor — the file-loading suites (the Main.qml harness
    # targets: tst_palette_pins, tst_command_availability,
    # tst_canvas_context_menu, tst_chord_table) key-echo and fail. Stage
    # an identical second copy flat in the build dir so every flat test
    # binary keeps its documented exe-adjacent resolution with zero
    # per-test knowledge. Guarded by APPNAME_BUNDLE: in the default
    # (bundle-OFF) config $<TARGET_FILE_DIR:appname-shell> IS this
    # directory and the block generates nothing — build files are
    # byte-unchanged.
    if(APPNAME_BUNDLE)
        add_custom_command(TARGET appname-shell POST_BUILD
            COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_CURRENT_BINARY_DIR}/i18n"
            COMMAND ${CMAKE_COMMAND} -E copy_if_different
                    ${APPNAME_I18N_CATALOGS} "${CMAKE_CURRENT_BINARY_DIR}/i18n"
            COMMAND ${Python3_EXECUTABLE} "${CMAKE_CURRENT_SOURCE_DIR}/i18n/merge_catalogs.py"
                    "${APPNAME_REPO_ROOT}/i18n/en.json"
                    "${CMAKE_CURRENT_SOURCE_DIR}/i18n/ui.en.json"
                    "${CMAKE_CURRENT_BINARY_DIR}/i18n/en.json"
            COMMENT "Staging i18n catalogs flat in the build dir (bundle-ON: the test binaries' exe-adjacent i18n/)"
            VERBATIM)
    endif()
endif()

target_include_directories(appname-shell PRIVATE
    "${APPNAME_REPO_ROOT}/include"
    src
)
target_link_libraries(appname-shell PRIVATE
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Svg
    "${APPNAME_CABI_LIB}"
)
if(APPNAME_CABI_BUILD)
    add_dependencies(appname-shell appname_cabi_build)
endif()

# ---- unit tests (ABI-free: catalog grammar, samples blob, envelopes) -------
# ---- BUILD COMMIT, CAPTURED FRESH EVERY BUILD (B-09) -----------------
# B-09's row asks for "version + commit". The VERSION is a policy question
# (three declared versions, none of them the v1.0.2 release — C-708) and
# waits on F-01/F-05. The COMMIT is a FACT, so it is buildable now.
#
# Runs EVERY build and rewrites the header only when the sha CHANGES, so a
# recompile happens exactly when the truth does. It is deliberately NOT
# anchored on a file: see cmake/write_build_info.cmake for why the usual
# `.git/HEAD` idiom is doubly wrong in a worktree.
set(APPNAME_BUILD_INFO_H "${CMAKE_CURRENT_BINARY_DIR}/build_info.h")
add_custom_target(appname_build_info ALL
    COMMAND ${CMAKE_COMMAND}
            "-DSRC=${APPNAME_REPO_ROOT}" "-DOUT=${APPNAME_BUILD_INFO_H}"
            -P "${CMAKE_CURRENT_SOURCE_DIR}/cmake/write_build_info.cmake"
    BYPRODUCTS "${APPNAME_BUILD_INFO_H}"
    COMMENT "build_info.h: capturing the build's own commit")
add_dependencies(appname-shell appname_build_info)
target_include_directories(appname-shell PRIVATE "${CMAKE_CURRENT_BINARY_DIR}")
# RC3 (operator WORD 36 (a), sitting S-03): the About dialog shows the
# version the build ALREADY carries -- the one APPNAME_VERSION parsed from
# Cargo.toml above, which also stamps the bundle's CFBundleShortVersionString.
# One source; nothing re-declared.
target_compile_definitions(appname-shell PRIVATE
    APPNAME_VERSION_STRING="${APPNAME_VERSION}")

enable_testing()

foreach(t tst_sample_blob tst_i18n_catalog tst_envelope_builder
        tst_sample_blob_adversarial tst_i18n_catalog_adversarial
        tst_envelope_builder_edges tst_theme_contrast
        tst_recovery_dialog_catalog tst_vector_path
        tst_clipboard_ops tst_text_builders
        tst_transform_builders tst_filter_builders tst_font_meta
        tst_filter_table tst_adjustment_table tst_announcer tst_a11y_tree_lint
        tst_selection_scan tst_focus_probes tst_selection_gestures
        tst_units tst_review_dialog_probes tst_delta_review_probes
        tst_repaint_scoping tst_brush_hardness_default
        tst_gesture_net tst_crop_capability_pin
        tst_dirty_flag tst_qd30_dialog_render tst_mask_feather
        tst_about_credits tst_about_render tst_license_texts
        tst_licenses_dialog)
    qt_add_executable(${t} tests/${t}.cpp)
    target_include_directories(${t} PRIVATE src)
    target_link_libraries(${t} PRIVATE Qt6::Core Qt6::Gui Qt6::Test)
    set_target_properties(${t} PROPERTIES MACOSX_BUNDLE FALSE)
    add_test(NAME ${t} COMMAND ${t})
endforeach()

# Q-D30 RENDERING PIN: the real AdjustmentParamDialog booted in the
# PSEUDO locale — footer buttons must render the catalog, never Qt's
# hard-English pair; plus the zero-standardButtons class pin (see
# tests/tst_qd30_dialog_render.cpp). No engine link: rendering needs
# the catalog and Theme alone.
target_sources(tst_qd30_dialog_render PRIVATE
    src/i18n_catalog.cpp src/i18n_catalog.h)
target_compile_definitions(tst_qd30_dialog_render PRIVATE
    APPNAME_SHELL_DIR="${CMAKE_CURRENT_SOURCE_DIR}"
    APPNAME_REPO_I18N_DIR="${APPNAME_REPO_ROOT}/i18n")
target_link_libraries(tst_qd30_dialog_render PRIVATE
    Qt6::Qml Qt6::Quick Qt6::QuickControls2)

# B-09 ACCEPTANCE, MADE EXECUTABLE: the About dialog's credit SET against
# the obligations THIRD-PARTY-NOTICES.md records for THIS surface. Nothing
# else checked the "matches THIRD-PARTY-NOTICES" half of the row, so the
# dialog could lose a credit — or the notices could gain an obligation —
# with every suite staying green. Reads two repo files; no QML boot.
target_sources(tst_about_credits PRIVATE src/i18n_catalog.cpp src/i18n_catalog.h)
target_compile_definitions(tst_about_credits PRIVATE
    APPNAME_SHELL_DIR="${CMAKE_CURRENT_SOURCE_DIR}"
    APPNAME_REPO_I18N_DIR="${APPNAME_REPO_ROOT}/i18n"
    APPNAME_REPO_ROOT="${APPNAME_REPO_ROOT}")

# B-09 "About SHOWS all": the REAL dialog booted and OPENED against the
# MERGED catalog (canonical + shell seed) — the one that ships. Three of
# its ids live only in the seed, so a canonical-only boot would key-echo
# them and the test would assert against a catalog no user sees.
target_sources(tst_about_render PRIVATE src/i18n_catalog.cpp src/i18n_catalog.h)
target_compile_definitions(tst_about_render PRIVATE
    APPNAME_SHELL_DIR="${CMAKE_CURRENT_SOURCE_DIR}"
    APPNAME_REPO_I18N_DIR="${APPNAME_REPO_ROOT}/i18n")
target_link_libraries(tst_about_render PRIVATE
    Qt6::Qml Qt6::Quick Qt6::QuickControls2)

# B-09 "the texts REACH a user": the real LicensesDialog booted over a real
# LicenseTexts on a fixture that mirrors packaging. tst_license_texts pins
# the C++ walk; this pins that its result crosses into QML and renders --
# and the unavailable arm, which is the state every dev build is in.
target_sources(tst_licenses_dialog PRIVATE
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/license_texts.cpp src/license_texts.h)
target_compile_definitions(tst_licenses_dialog PRIVATE
    APPNAME_SHELL_DIR="${CMAKE_CURRENT_SOURCE_DIR}"
    APPNAME_REPO_I18N_DIR="${APPNAME_REPO_ROOT}/i18n"
    APPNAME_REPO_ROOT="${APPNAME_REPO_ROOT}")
target_link_libraries(tst_licenses_dialog PRIVATE
    Qt6::Qml Qt6::Quick Qt6::QuickControls2)

target_sources(tst_license_texts PRIVATE src/license_texts.cpp src/license_texts.h)
target_compile_definitions(tst_license_texts PRIVATE
    APPNAME_REPO_ROOT="${APPNAME_REPO_ROOT}")

target_sources(tst_sample_blob PRIVATE src/sample_blob.cpp src/sample_blob.h)
target_sources(tst_i18n_catalog PRIVATE src/i18n_catalog.cpp src/i18n_catalog.h)
target_sources(tst_envelope_builder PRIVATE src/envelope_builder.cpp src/envelope_builder.h)
target_sources(tst_sample_blob_adversarial PRIVATE src/sample_blob.cpp src/sample_blob.h)
target_sources(tst_i18n_catalog_adversarial PRIVATE src/i18n_catalog.cpp src/i18n_catalog.h)
target_sources(tst_envelope_builder_edges PRIVATE src/envelope_builder.cpp src/envelope_builder.h)
target_sources(tst_vector_path PRIVATE src/envelope_builder.cpp src/envelope_builder.h)
target_sources(tst_clipboard_ops PRIVATE src/clipboard_ops.cpp src/clipboard_ops.h)
target_sources(tst_text_builders PRIVATE src/envelope_builder.cpp src/envelope_builder.h)
target_sources(tst_transform_builders PRIVATE src/envelope_builder.cpp src/envelope_builder.h)
target_sources(tst_filter_builders PRIVATE src/envelope_builder.cpp src/envelope_builder.h)
target_sources(tst_font_meta PRIVATE src/font_meta.cpp src/font_meta.h)

# Reviewer-added invariant pin (P1 slice review): recomputes WCAG contrast
# over the hex tokens parsed out of the checked-in Theme.qml — a token edit
# that drops a rendered pairing below its documented bar fails here.
target_compile_definitions(tst_theme_contrast PRIVATE
    APPNAME_THEME_QML_PATH="${CMAKE_CURRENT_SOURCE_DIR}/qml/Theme.qml")

# Reviewer-specified mechanical pin (A11Y-1 MAJOR-1): the filter dialog's
# param table diffs against the ENGINE's registered schema strings — the
# sigmaPx drift class reds mechanically.
target_compile_definitions(tst_filter_table PRIVATE
    APPNAME_FILTER_DIALOG_QML="${CMAKE_CURRENT_SOURCE_DIR}/qml/dialogs/FilterParamDialog.qml"
    APPNAME_FILTER_CORPUS_DIR="${APPNAME_REPO_ROOT}/crates/appname-filters/src/corpus")
# item 4 class (a): the adjustment twin of the filter-table pin.
target_compile_definitions(tst_adjustment_table PRIVATE
    APPNAME_ADJUST_DIALOG_QML="${CMAKE_CURRENT_SOURCE_DIR}/qml/dialogs/AdjustmentParamDialog.qml"
    APPNAME_ADJUST_OPS_RS="${APPNAME_REPO_ROOT}/crates/appname-adjust/src/ops.rs"
    APPNAME_CURVE_EDITOR_QML="${CMAKE_CURRENT_SOURCE_DIR}/qml/dialogs/CurveEditor.qml")

# Reviewer-added invariant pin (P6 slice review): parses the AppI18n.tr
# call sites out of the checked-in RecoveryDialog.qml and renders each
# against every checked-in locale catalog — a caller/catalog placeholder
# mismatch (literal "{count}" or an unconsumed " count=7" append in the
# crash prompt) fails here; no other check covers that seam.
target_sources(tst_recovery_dialog_catalog PRIVATE src/i18n_catalog.cpp src/i18n_catalog.h)
target_compile_definitions(tst_recovery_dialog_catalog PRIVATE
    APPNAME_RECOVERY_DIALOG_QML_PATH="${CMAKE_CURRENT_SOURCE_DIR}/qml/dialogs/RecoveryDialog.qml"
    APPNAME_REPO_I18N_DIR="${APPNAME_REPO_ROOT}/i18n")

# A11Y-3: the §31.3 polite live region — rate limit (500 ms, precise),
# latest-wins per source, FIFO across sources, the 700 ms KS bound (rule
# 7), catalog-resolution goldens against the REAL checked-in en.json, and
# the uncataloged-id refusal gate (never key-echo into speech).
target_sources(tst_announcer PRIVATE
    src/announcer.cpp src/announcer.h src/i18n_catalog.cpp src/i18n_catalog.h)
# C-204: the history-label pin DERIVES its population by parsing the
# setStatusFromResult call sites out of the checked-in controller source
# (read as TEXT at run time — this target does not compile or link it),
# so a newly hand-written kind changes the derived set instead of
# slipping past a fixed list. Same shape as tst_recovery_dialog_catalog
# and tst_filter_table above: a checked-in source file passed by path.
target_compile_definitions(tst_announcer PRIVATE
    APPNAME_REPO_I18N_DIR="${APPNAME_REPO_ROOT}/i18n"
    APPNAME_DOCUMENT_CONTROLLER_CPP="${CMAKE_CURRENT_SOURCE_DIR}/src/document_controller.cpp")
# Pin 7 (review D1) constructs a QQuickWindow on the offscreen platform.
target_link_libraries(tst_announcer PRIVATE Qt6::Quick)

# A11Y-3 wave B: the pure strip accumulator behind the selection-bounds
# announcement (each readback ≤ 2^26 px by construction).
target_sources(tst_selection_scan PRIVATE
    src/selection_scan.cpp src/selection_scan.h)

# v1.0.1 items 2+3: the pure unit-conversion helper + preset table behind
# the New Document / Resize unit-aware dialogs (exact px pins).
target_sources(tst_units PRIVATE src/units.cpp src/units.h)

# A11Y-5 hook C: the durable A11Y-2 probe scenes (offscreen, Basic
# style) — re-runnable KS evidence for the O-QA junction.
target_link_libraries(tst_focus_probes PRIVATE
    Qt6::Qml Qt6::Quick Qt6::QuickControls2)
target_compile_definitions(tst_focus_probes PRIVATE
    APPNAME_PROBES_DIR="${CMAKE_CURRENT_SOURCE_DIR}/tests/probes")

# v1.0.1 item 6: the gesture-level selection net — REAL synthesized
# mouse events against a REAL engine document (links the cabi; the
# only test target that does — the defect class it guards is invisible
# to both the ABI-free unit tests and the command-driven tour).
target_sources(tst_selection_gestures PRIVATE
    src/abi_client.cpp src/abi_client.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/announcer.cpp src/announcer.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h)
target_include_directories(tst_selection_gestures PRIVATE
    "${APPNAME_REPO_ROOT}/include")
target_link_libraries(tst_selection_gestures PRIVATE
    Qt6::Qml Qt6::Quick "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_selection_gestures appname_cabi_build)
endif()

# Pro-Wave row 6 — THE §11.2 MASK PROPERTY NET (tests/
# tst_mask_feather.cpp). Same cabi-linked idiom as
# tst_selection_gestures / tst_auto_adjust (no QML module: row 6 ships
# no chrome — every property-control label is uncataloged and the
# controls are HELD, see heldOnI18nUntilCatalogRowsLand). Drives
# mask.set_props through the REAL applyCommand door against a REAL
# engine document and asserts engine truth: composite pixels across a
# hard mask step, the sigma = feather_px/3 REACH (where the stale
# sigma = feather_px reading moves a measurable boundary), the typed
# out-of-range refusals, and the FI-18 absence pinned as a tripwire.
target_sources(tst_mask_feather PRIVATE
    src/abi_client.cpp src/abi_client.h
    src/announcer.cpp src/announcer.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h)
target_include_directories(tst_mask_feather PRIVATE
    "${APPNAME_REPO_ROOT}/include")
target_link_libraries(tst_mask_feather PRIVATE
    Qt6::Qml Qt6::Quick "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_mask_feather appname_cabi_build)
endif()
# The staged exe-adjacent i18n/ (appname-shell POST_BUILD) is this
# test's catalog source — same resolution order as the app. The i18n
# hold this net pins is only meaningful against the REAL catalogs.
add_dependencies(tst_mask_feather appname-shell)

# Repaint-scoping slice (perf sweep F1+F2+F4): commit-path invalidation
# truth on a REAL CanvasView + engine document — mask-read counts per
# commit class (the F1 revision gate + F4 shared-read discriminators),
# partial-vs-full repaint scoping (F2 per-path table), the windowed-ants
# pan escape (F1c), and the announcement byte-equality pin. Same
# cabi-linked registration idiom as tst_selection_gestures.
target_sources(tst_repaint_scoping PRIVATE
    src/abi_client.cpp src/abi_client.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/announcer.cpp src/announcer.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h)
target_include_directories(tst_repaint_scoping PRIVATE
    "${APPNAME_REPO_ROOT}/include")
target_link_libraries(tst_repaint_scoping PRIVATE
    Qt6::Qml Qt6::Quick "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_repaint_scoping appname_cabi_build)
endif()

# O-29 (item 4 follow-up): the hardness-omission net — an unstated brush
# hardness row must emit NO `hardness` param so the engine's preset
# hardness governs (brush.stroke.yaml gives the param no schema default).
# Needs a REAL engine document AND the shell's own envelope builder: the
# trap case swaps the preset to hard-round-16, which no shell surface can
# reach yet. Same cabi-linked registration idiom as tst_repaint_scoping.
target_sources(tst_brush_hardness_default PRIVATE
    src/abi_client.cpp src/abi_client.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/announcer.cpp src/announcer.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h)
target_include_directories(tst_brush_hardness_default PRIVATE
    "${APPNAME_REPO_ROOT}/include")
target_link_libraries(tst_brush_hardness_default PRIVATE
    Qt6::Qml Qt6::Quick "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_brush_hardness_default appname_cabi_build)
endif()

# (4)-3: the dirty flag needs a REAL engine document — doc.new, one
# journalled command, a package save, and an undo back to the baseline —
# so it takes the same cabi-linked source set as tst_brush_hardness_default.
target_sources(tst_dirty_flag PRIVATE
    src/abi_client.cpp src/abi_client.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/announcer.cpp src/announcer.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h)
target_include_directories(tst_dirty_flag PRIVATE
    "${APPNAME_REPO_ROOT}/include")
target_link_libraries(tst_dirty_flag PRIVATE
    Qt6::Qml Qt6::Quick "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_dirty_flag appname_cabi_build)
endif()

# §5 REVIEW PROBES (v1.0.1 items 2+3) — reviewer harness, lane-adopted:
# drives the REAL dialog QML with synthesized events against a REAL
# engine document (see tests/tst_review_dialog_probes.cpp).
target_sources(tst_review_dialog_probes PRIVATE
    src/abi_client.cpp src/abi_client.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/announcer.cpp src/announcer.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h
    src/units.cpp src/units.h)
target_include_directories(tst_review_dialog_probes PRIVATE
    "${APPNAME_REPO_ROOT}/include")
target_compile_definitions(tst_review_dialog_probes PRIVATE
    APPNAME_SHELL_DIR="${CMAKE_CURRENT_SOURCE_DIR}"
    APPNAME_REPO_I18N_DIR="${APPNAME_REPO_ROOT}/i18n")
target_link_libraries(tst_review_dialog_probes PRIVATE
    Qt6::Qml Qt6::Quick Qt6::QuickControls2 "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_review_dialog_probes appname_cabi_build)
endif()

# §5 DELTA RE-REVIEW PROBES (items 2+3 fix round) — the delta reviewer's
# independent variations, adopted by the lane: D1 positive controls
# (valid-ppi paths live, so the guard pass is non-vacuous), the exact
# 2^32+131 Resize wrap probe, and the LOAD-BEARING D5 preview-clamp
# assertion (the committed D5 slot records without asserting).
target_sources(tst_delta_review_probes PRIVATE
    src/abi_client.cpp src/abi_client.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/announcer.cpp src/announcer.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h
    src/units.cpp src/units.h)
target_include_directories(tst_delta_review_probes PRIVATE
    "${APPNAME_REPO_ROOT}/include")
target_compile_definitions(tst_delta_review_probes PRIVATE
    APPNAME_SHELL_DIR="${CMAKE_CURRENT_SOURCE_DIR}"
    APPNAME_REPO_I18N_DIR="${APPNAME_REPO_ROOT}/i18n")
target_link_libraries(tst_delta_review_probes PRIVATE
    Qt6::Qml Qt6::Quick Qt6::QuickControls2 "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_delta_review_probes appname_cabi_build)
endif()

# C-130 CROP CAPABILITY PIN — proves the shell behaves correctly WHEN the
# §19.1 non-destructive arm goes live (abi_minor 6 -> 7), BEFORE it goes
# live, and that today's locked-destructive behaviour is unchanged. Drives
# the REAL CropDialog.qml against a REAL engine document, with AppDoc
# bound to a stub that delegates the real controller and overrides only
# cropNonDestructiveLive (the context-property seam) while spying the
# doc.crop envelope. See tests/tst_crop_capability_pin.cpp.
target_sources(tst_crop_capability_pin PRIVATE
    src/abi_client.cpp src/abi_client.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/announcer.cpp src/announcer.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h
    src/units.cpp src/units.h)
target_include_directories(tst_crop_capability_pin PRIVATE
    "${APPNAME_REPO_ROOT}/include")
target_compile_definitions(tst_crop_capability_pin PRIVATE
    APPNAME_SHELL_DIR="${CMAKE_CURRENT_SOURCE_DIR}"
    APPNAME_REPO_I18N_DIR="${APPNAME_REPO_ROOT}/i18n")
target_link_libraries(tst_crop_capability_pin PRIVATE
    Qt6::Qml Qt6::Quick Qt6::QuickControls2 "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_crop_capability_pin appname_cabi_build)
endif()

# A11Y-3: mechanical §31.5-item-1 lint — sweeps the checked-in shell/qml
# for interactive controls exposing neither text nor Accessible.name; the
# wave-B holds ride an explicit expected-gap roster (two-sided: a roster
# row that stops matching an unlabeled control reds too, so the roster
# can only shrink honestly).
target_compile_definitions(tst_a11y_tree_lint PRIVATE
    APPNAME_QML_DIR="${CMAKE_CURRENT_SOURCE_DIR}/qml")

# v1.0.1 item 11b §5 DELTA RE-REVIEW pins (reviewer-added, UNCOMMITTED
# pending lane adoption): boots the REAL Main.qml offscreen — this test
# target compiles its own copy of the AppnameShell QML module from the
# same checked-in sources (a compiled-in module is only loadable from
# the binary that carries it) — and drives synthesized key/mouse events
# against the 11b fix round: D1 Esc-to-canvas + flyout popup semantics,
# D2 grouped-slot descriptions + no-key-opens-flyout + the eight tool
# letters, D4 the 600 ms hold (runtime mechanism + a source pin that
# main.cpp sets it BEFORE the QML load), D5 fixed max-over-members
# width (no ratchet; measure row hidden/unfocusable/absent from a11y),
# D6 click-announces / keyboard-activation-suppressed, D7 menu door,
# and the pane-cycle/faces/flyout framework regressions.
qt_add_executable(tst_palette_pins
    tests/tst_palette_pins.cpp
    src/units.cpp src/units.h
    src/abi_client.cpp src/abi_client.h
    src/announcer.cpp src/announcer.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/icon_provider.cpp src/icon_provider.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/layer_thumb_provider.cpp src/layer_thumb_provider.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h
    # v1.0.1 item 11e: the harness mirrors main.cpp, and the letter
    # chords now LIVE in this dispatcher (the Main.qml Shortcuts are
    # gone), so the suite must build and attach it or every letter case
    # would pass vacuously against a dead keymap.
    src/tool_key_dispatcher.cpp src/tool_key_dispatcher.h
)
qt_add_qml_module(tst_palette_pins
    URI AppnameShell
    VERSION 1.0
    OUTPUT_DIRECTORY probe-qml/AppnameShell
    QML_FILES
        qml/Theme.qml
        qml/Main.qml
        qml/CanvasContextMenu.qml
        qml/ChordMenuItem.qml
        qml/ToolPalette.qml
        qml/LayersPanel.qml
        qml/HistoryPanel.qml
        qml/MaskPanel.qml
        qml/NewDocumentDialog.qml
        qml/dialogs/AdjustmentParamDialog.qml
        qml/dialogs/CurveEditor.qml
        qml/dialogs/AssignProfileDialog.qml
        qml/dialogs/RecoveryDialog.qml
        qml/dialogs/CanvasSizeDialog.qml
        qml/dialogs/ConvertProfileDialog.qml
        qml/dialogs/CropDialog.qml
        qml/dialogs/DepthConvertDialog.qml
        qml/dialogs/FillDialog.qml
        qml/dialogs/FlattenConfirmDialog.qml
        qml/dialogs/DeleteConfirmDialog.qml
        qml/dialogs/AboutDialog.qml
        qml/dialogs/LicensesDialog.qml
        qml/dialogs/DiscardConfirmDialog.qml
        qml/dialogs/ResizeDialog.qml
        qml/dialogs/RotateDialog.qml
        qml/dialogs/ModeConvertDialog.qml
        qml/dialogs/FilterParamDialog.qml
        qml/dialogs/SelectionModifyDialog.qml
        qml/dialogs/TrimDialog.qml
)
qt_add_resources(tst_palette_pins "lucide_icons_pins"
    PREFIX "/icons/lucide"
    BASE "icons/lucide"
    FILES ${APPNAME_LUCIDE_SVGS} icons/lucide/LICENSE)
set_target_properties(tst_palette_pins PROPERTIES MACOSX_BUNDLE FALSE)
target_include_directories(tst_palette_pins PRIVATE
    "${APPNAME_REPO_ROOT}/include" src)
target_link_libraries(tst_palette_pins PRIVATE
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Svg
    Qt6::Test "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_palette_pins appname_cabi_build)
endif()
# The staged exe-adjacent i18n/ (appname-shell POST_BUILD) is this
# test's catalog source — same resolution order as the app.
add_dependencies(tst_palette_pins appname-shell)
target_compile_definitions(tst_palette_pins PRIVATE
    APPNAME_SHELL_MAIN_CPP="${CMAKE_CURRENT_SOURCE_DIR}/src/main.cpp")
add_test(NAME tst_palette_pins COMMAND tst_palette_pins)

# v1.0.1 item 14a — THE DESK CHECKPOINT NET: the command-availability
# state matrix (tests/tst_command_availability.cpp). Boots the REAL
# Main.qml offscreen (its own compiled copy of the AppnameShell module,
# the tst_palette_pins harness) and asserts, per migrated action across
# the document-state matrix: frozen OLD predicate == availability row
# == the actual MenuItem's enabled — plus the reasonId catalog contract
# and the setActiveLayerIndex re-notify pin.
qt_add_executable(tst_command_availability
    tests/tst_command_availability.cpp
    src/units.cpp src/units.h
    src/abi_client.cpp src/abi_client.h
    src/announcer.cpp src/announcer.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/icon_provider.cpp src/icon_provider.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/layer_thumb_provider.cpp src/layer_thumb_provider.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h
)
qt_add_qml_module(tst_command_availability
    URI AppnameShell
    VERSION 1.0
    OUTPUT_DIRECTORY probe-qml-avail/AppnameShell
    QML_FILES
        qml/Theme.qml
        qml/Main.qml
        qml/CanvasContextMenu.qml
        qml/ChordMenuItem.qml
        qml/ToolPalette.qml
        qml/LayersPanel.qml
        qml/HistoryPanel.qml
        qml/MaskPanel.qml
        qml/NewDocumentDialog.qml
        qml/dialogs/AdjustmentParamDialog.qml
        qml/dialogs/CurveEditor.qml
        qml/dialogs/AssignProfileDialog.qml
        qml/dialogs/RecoveryDialog.qml
        qml/dialogs/CanvasSizeDialog.qml
        qml/dialogs/ConvertProfileDialog.qml
        qml/dialogs/CropDialog.qml
        qml/dialogs/DepthConvertDialog.qml
        qml/dialogs/FillDialog.qml
        qml/dialogs/FlattenConfirmDialog.qml
        qml/dialogs/DeleteConfirmDialog.qml
        qml/dialogs/AboutDialog.qml
        qml/dialogs/LicensesDialog.qml
        qml/dialogs/DiscardConfirmDialog.qml
        qml/dialogs/ResizeDialog.qml
        qml/dialogs/RotateDialog.qml
        qml/dialogs/ModeConvertDialog.qml
        qml/dialogs/FilterParamDialog.qml
        qml/dialogs/SelectionModifyDialog.qml
        qml/dialogs/TrimDialog.qml
)
qt_add_resources(tst_command_availability "lucide_icons_avail"
    PREFIX "/icons/lucide"
    BASE "icons/lucide"
    FILES ${APPNAME_LUCIDE_SVGS} icons/lucide/LICENSE)
set_target_properties(tst_command_availability PROPERTIES MACOSX_BUNDLE FALSE)
target_include_directories(tst_command_availability PRIVATE
    "${APPNAME_REPO_ROOT}/include" src)
target_link_libraries(tst_command_availability PRIVATE
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Svg
    Qt6::Test "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_command_availability appname_cabi_build)
endif()
# The staged exe-adjacent i18n/ (appname-shell POST_BUILD) is this
# test's catalog source — same resolution order as the app.
add_dependencies(tst_command_availability appname-shell)
add_test(NAME tst_command_availability COMMAND tst_command_availability)

# v1.0.1 item 7 — THE CANVAS CONTEXT MENU NET (tests/
# tst_canvas_context_menu.cpp). Boots the REAL Main.qml offscreen (its
# own compiled copy of the AppnameShell module — the tst_palette_pins
# harness) against a REAL engine document and drives the three REAL
# open paths (synthesized right-click, Shift+F10, the Menu key), the
# section truth, enabled == the 14a availability surface, and
# action-identity vs the top-menu twins by engine readback.
qt_add_executable(tst_canvas_context_menu
    tests/tst_canvas_context_menu.cpp
    src/units.cpp src/units.h
    src/abi_client.cpp src/abi_client.h
    src/announcer.cpp src/announcer.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/icon_provider.cpp src/icon_provider.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/layer_thumb_provider.cpp src/layer_thumb_provider.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h
)
qt_add_qml_module(tst_canvas_context_menu
    URI AppnameShell
    VERSION 1.0
    OUTPUT_DIRECTORY probe-qml-ctxmenu/AppnameShell
    QML_FILES
        qml/Theme.qml
        qml/Main.qml
        qml/CanvasContextMenu.qml
        qml/ChordMenuItem.qml
        qml/ToolPalette.qml
        qml/LayersPanel.qml
        qml/HistoryPanel.qml
        qml/MaskPanel.qml
        qml/NewDocumentDialog.qml
        qml/dialogs/AdjustmentParamDialog.qml
        qml/dialogs/CurveEditor.qml
        qml/dialogs/AssignProfileDialog.qml
        qml/dialogs/RecoveryDialog.qml
        qml/dialogs/CanvasSizeDialog.qml
        qml/dialogs/ConvertProfileDialog.qml
        qml/dialogs/CropDialog.qml
        qml/dialogs/DepthConvertDialog.qml
        qml/dialogs/FillDialog.qml
        qml/dialogs/FlattenConfirmDialog.qml
        qml/dialogs/DeleteConfirmDialog.qml
        qml/dialogs/AboutDialog.qml
        qml/dialogs/LicensesDialog.qml
        qml/dialogs/DiscardConfirmDialog.qml
        qml/dialogs/ResizeDialog.qml
        qml/dialogs/RotateDialog.qml
        qml/dialogs/ModeConvertDialog.qml
        qml/dialogs/FilterParamDialog.qml
        qml/dialogs/SelectionModifyDialog.qml
        qml/dialogs/TrimDialog.qml
)
qt_add_resources(tst_canvas_context_menu "lucide_icons_ctxmenu"
    PREFIX "/icons/lucide"
    BASE "icons/lucide"
    FILES ${APPNAME_LUCIDE_SVGS} icons/lucide/LICENSE)
set_target_properties(tst_canvas_context_menu PROPERTIES MACOSX_BUNDLE FALSE)
target_include_directories(tst_canvas_context_menu PRIVATE
    "${APPNAME_REPO_ROOT}/include" src)
target_link_libraries(tst_canvas_context_menu PRIVATE
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Svg
    Qt6::Test "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_canvas_context_menu appname_cabi_build)
endif()
# The staged exe-adjacent i18n/ (appname-shell POST_BUILD) is this
# test's catalog source — same resolution order as the app.
add_dependencies(tst_canvas_context_menu appname-shell)
add_test(NAME tst_canvas_context_menu COMMAND tst_canvas_context_menu)

# v1.0.1 item 11d Phase 1 — THE OPTIONS BAR NET (tests/
# tst_options_bar.cpp) and THE PANE-CYCLE MATRIX T1-T5 (tests/
# tst_pane_cycle.cpp). Both boot the REAL Main.qml offscreen (the
# item-7/14b harness — each binary compiles its own copy of the
# AppnameShell module) against REAL engine documents: the tool×control
# presence/absence matrix incl. the RESERVED absences, binding truth
# both directions with engine-truth spies (journal kinds, typed
# out-of-range refusals), T6 rehouse parity + the D3 announcement
# guard, the show_mesh pixel diff; and the CORRECTED cycle order
# [options, palette, canvas, panels] with the C2 empty-pane skip, the
# C3 Esc consumption, 200-Tab trap freedom, and the D4-R modal no-op.
# v1.0.1 — THE LAYERS-PANEL CREATION ROUTES (tests/tst_layers_mask_route.cpp,
# tests/tst_layers_fill_route.cpp). Both drive the Layers panel's action
# strip through its REAL onTriggered handlers and read the result back
# through the doc-info door, never from shell state. mask route: the
# five mask.add `from` modes, and the OCCUPIED SLOT left to the engine
# because the ABI's info mask object carries no `kind`. fill route: the
# layer.add{kind:'fill'} gradient presets, whose envelopes are the
# demo's own (demo/build-demo.jsonl lines 4-5), not invented here.
#
# THEY JOIN THIS foreach RATHER THAN CARRYING THEIR OWN BLOCKS. The mask
# suite shipped with a hand-copied 60-line duplicate of the
# tst_history_panel block; this list already parameterises every line of
# it on ${t}. One list, one place to add a QML file when Main.qml grows
# a new import — a copied block silently stops matching.
foreach(t tst_options_bar tst_pane_cycle tst_layers_mask_route
        tst_layers_fill_route tst_layers_blend_row tst_discard_gate)
    qt_add_executable(${t}
        tests/${t}.cpp
        src/units.cpp src/units.h
        src/abi_client.cpp src/abi_client.h
        src/announcer.cpp src/announcer.h
        src/canvas_view.cpp src/canvas_view.h
        src/clipboard_ops.cpp src/clipboard_ops.h
        src/document_controller.cpp src/document_controller.h
        src/envelope_builder.cpp src/envelope_builder.h
        src/font_meta.cpp src/font_meta.h
        src/icon_provider.cpp src/icon_provider.h
        src/i18n_catalog.cpp src/i18n_catalog.h
        src/layer_thumb_provider.cpp src/layer_thumb_provider.h
        src/sample_blob.cpp src/sample_blob.h
        src/selection_scan.cpp src/selection_scan.h
    )
    qt_add_qml_module(${t}
        URI AppnameShell
        VERSION 1.0
        OUTPUT_DIRECTORY probe-qml-${t}/AppnameShell
        QML_FILES
            qml/Theme.qml
            qml/Main.qml
            qml/CanvasContextMenu.qml
            qml/ChordMenuItem.qml
            qml/ToolPalette.qml
            qml/LayersPanel.qml
            qml/HistoryPanel.qml
            qml/MaskPanel.qml
            qml/NewDocumentDialog.qml
            qml/dialogs/AdjustmentParamDialog.qml
        qml/dialogs/CurveEditor.qml
        qml/dialogs/AssignProfileDialog.qml
            qml/dialogs/RecoveryDialog.qml
            qml/dialogs/CanvasSizeDialog.qml
            qml/dialogs/ConvertProfileDialog.qml
            qml/dialogs/CropDialog.qml
            qml/dialogs/DepthConvertDialog.qml
            qml/dialogs/FillDialog.qml
            qml/dialogs/FlattenConfirmDialog.qml
            qml/dialogs/DeleteConfirmDialog.qml
            qml/dialogs/AboutDialog.qml
            qml/dialogs/LicensesDialog.qml
            qml/dialogs/DiscardConfirmDialog.qml
            qml/dialogs/ResizeDialog.qml
            qml/dialogs/RotateDialog.qml
            qml/dialogs/ModeConvertDialog.qml
            qml/dialogs/FilterParamDialog.qml
            qml/dialogs/SelectionModifyDialog.qml
            qml/dialogs/TrimDialog.qml
    )
    qt_add_resources(${t} "lucide_icons_${t}"
        PREFIX "/icons/lucide"
        BASE "icons/lucide"
        FILES ${APPNAME_LUCIDE_SVGS} icons/lucide/LICENSE)
    set_target_properties(${t} PROPERTIES MACOSX_BUNDLE FALSE)
    target_include_directories(${t} PRIVATE
        "${APPNAME_REPO_ROOT}/include" src)
    target_link_libraries(${t} PRIVATE
        Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Svg
        Qt6::Test "${APPNAME_CABI_LIB}")
    if(APPNAME_CABI_BUILD)
        add_dependencies(${t} appname_cabi_build)
    endif()
    # The staged exe-adjacent i18n/ (appname-shell POST_BUILD) is this
    # test's catalog source — same resolution order as the app.
    add_dependencies(${t} appname-shell)
    add_test(NAME ${t} COMMAND ${t})
endforeach()
# Wave F — THE HISTORY-PANEL JUMP NET (tests/tst_history_panel.cpp).
# Boots the REAL Main.qml offscreen (its own compiled copy of the
# AppnameShell module — the tst_palette_pins harness) against a REAL
# engine document and drives the HistoryPanel rows with real click and
# key events: mid-row jump back, tail-row jump forward, keyboard
# activation, the † marker position mapping, the typed out-of-range
# refusal, and the by-design announcer silence.
qt_add_executable(tst_history_panel
    tests/tst_history_panel.cpp
    src/units.cpp src/units.h
    src/abi_client.cpp src/abi_client.h
    src/announcer.cpp src/announcer.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/icon_provider.cpp src/icon_provider.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/layer_thumb_provider.cpp src/layer_thumb_provider.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h
)
qt_add_qml_module(tst_history_panel
    URI AppnameShell
    VERSION 1.0
    OUTPUT_DIRECTORY probe-qml-histpanel/AppnameShell
    QML_FILES
        qml/Theme.qml
        qml/Main.qml
        qml/CanvasContextMenu.qml
        qml/ChordMenuItem.qml
        qml/ToolPalette.qml
        qml/LayersPanel.qml
        qml/HistoryPanel.qml
        qml/MaskPanel.qml
        qml/NewDocumentDialog.qml
        qml/dialogs/AdjustmentParamDialog.qml
        qml/dialogs/CurveEditor.qml
        qml/dialogs/AssignProfileDialog.qml
        qml/dialogs/RecoveryDialog.qml
        qml/dialogs/CanvasSizeDialog.qml
        qml/dialogs/ConvertProfileDialog.qml
        qml/dialogs/CropDialog.qml
        qml/dialogs/DeleteConfirmDialog.qml
        qml/dialogs/AboutDialog.qml
        qml/dialogs/LicensesDialog.qml
        qml/dialogs/DiscardConfirmDialog.qml
        qml/dialogs/DepthConvertDialog.qml
        qml/dialogs/FillDialog.qml
        qml/dialogs/FlattenConfirmDialog.qml
        qml/dialogs/DeleteConfirmDialog.qml
        qml/dialogs/ResizeDialog.qml
        qml/dialogs/RotateDialog.qml
        qml/dialogs/ModeConvertDialog.qml
        qml/dialogs/FilterParamDialog.qml
        qml/dialogs/SelectionModifyDialog.qml
        qml/dialogs/TrimDialog.qml
)
qt_add_resources(tst_history_panel "lucide_icons_histpanel"
    PREFIX "/icons/lucide"
    BASE "icons/lucide"
    FILES ${APPNAME_LUCIDE_SVGS} icons/lucide/LICENSE)
set_target_properties(tst_history_panel PROPERTIES MACOSX_BUNDLE FALSE)
target_include_directories(tst_history_panel PRIVATE
    "${APPNAME_REPO_ROOT}/include" src)
target_link_libraries(tst_history_panel PRIVATE
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Svg
    Qt6::Test "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_history_panel appname_cabi_build)
endif()
# The staged exe-adjacent i18n/ (appname-shell POST_BUILD) is this
# test's catalog source — same resolution order as the app.
add_dependencies(tst_history_panel appname-shell)
add_test(NAME tst_history_panel COMMAND tst_history_panel)


# §17.2 AUTO TONE / CONTRAST / COLOR — THE SHELL-HALF WIRING NET
# (tests/tst_auto_adjust_menu.cpp). Boots the REAL Main.qml offscreen
# (its own compiled copy of the AppnameShell module — the
# tst_palette_pins harness) against a REAL engine document and drives
# the three new Image-menu rows through their REAL onTriggered handlers:
# the journal grows by exactly one entry of exactly the expected kind,
# the status line and the HistoryPanel row's Accessible.name render the
# CAPTIONS, and a second Main.qml booted against the same catalog minus
# exactly `history.image.auto_tone` makes the key-echo probe FIRE on
# that same accessible name — the miss this slice was designed against,
# reproduced rather than argued.
qt_add_executable(tst_auto_adjust_menu
    tests/tst_auto_adjust_menu.cpp
    src/units.cpp src/units.h
    src/abi_client.cpp src/abi_client.h
    src/announcer.cpp src/announcer.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/icon_provider.cpp src/icon_provider.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/layer_thumb_provider.cpp src/layer_thumb_provider.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h
)
qt_add_qml_module(tst_auto_adjust_menu
    URI AppnameShell
    VERSION 1.0
    OUTPUT_DIRECTORY probe-qml-autoadjust/AppnameShell
    QML_FILES
        qml/Theme.qml
        qml/Main.qml
        qml/CanvasContextMenu.qml
        qml/ChordMenuItem.qml
        qml/ToolPalette.qml
        qml/LayersPanel.qml
        qml/HistoryPanel.qml
        qml/MaskPanel.qml
        qml/NewDocumentDialog.qml
        qml/dialogs/AdjustmentParamDialog.qml
        qml/dialogs/CurveEditor.qml
        qml/dialogs/AssignProfileDialog.qml
        qml/dialogs/RecoveryDialog.qml
        qml/dialogs/CanvasSizeDialog.qml
        qml/dialogs/ConvertProfileDialog.qml
        qml/dialogs/CropDialog.qml
        qml/dialogs/DeleteConfirmDialog.qml
        qml/dialogs/AboutDialog.qml
        qml/dialogs/LicensesDialog.qml
        qml/dialogs/DiscardConfirmDialog.qml
        qml/dialogs/DepthConvertDialog.qml
        qml/dialogs/FillDialog.qml
        qml/dialogs/FlattenConfirmDialog.qml
        qml/dialogs/ResizeDialog.qml
        qml/dialogs/RotateDialog.qml
        qml/dialogs/ModeConvertDialog.qml
        qml/dialogs/FilterParamDialog.qml
        qml/dialogs/SelectionModifyDialog.qml
        qml/dialogs/TrimDialog.qml
)
qt_add_resources(tst_auto_adjust_menu "lucide_icons_autoadjust"
    PREFIX "/icons/lucide"
    BASE "icons/lucide"
    FILES ${APPNAME_LUCIDE_SVGS} icons/lucide/LICENSE)
set_target_properties(tst_auto_adjust_menu PROPERTIES MACOSX_BUNDLE FALSE)
target_include_directories(tst_auto_adjust_menu PRIVATE
    "${APPNAME_REPO_ROOT}/include" src)
target_link_libraries(tst_auto_adjust_menu PRIVATE
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Svg
    Qt6::Test "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_auto_adjust_menu appname_cabi_build)
endif()
# The staged exe-adjacent i18n/ (appname-shell POST_BUILD) is this
# test's catalog source — same resolution order as the app, and the
# same bytes the echo probe mutates a copy of.
add_dependencies(tst_auto_adjust_menu appname-shell)
add_test(NAME tst_auto_adjust_menu COMMAND tst_auto_adjust_menu)

# v1.0.1 item 14b — THE CHORD TABLE NET (tests/tst_chord_table.cpp).
# Boots the REAL Main.qml offscreen (its own compiled copy of the
# AppnameShell module — the item-7 harness) against REAL engine
# documents and drives every new chord with real key events: Cmd+T
# frame entry, the Delete disposition (both arms + the §32.5 confirm
# dialog + coverage-scaled §13.1 truth), the Alt/Cmd+Delete engine-
# constant fills (R15 discriminated), Cmd+Shift+D reselect memory
# (mask byte-compare + the disabled-state leg), Cmd+J both arms,
# arrow nudges on the integer arm, the §30.2 text-field yield, the
# ChordMenuItem display truth, and the KEY-LAYER slice's ES:16452
# single-key suppression + Appendix-C T=Type letter.
# v1.0.1 item 11e appends: the letter chords on a REAL key-event
# dispatcher — the ES:16452 guard surviving the migration (including a
# permanent no-yield surface that claims no ShortcutOverride, so the
# guard is proven doing real work rather than riding the platform's own
# yield), the ES:16446 capture gate that stops a mid-drag letter from
# cancelling the gesture (with the click-session scope limit pinned as
# measured), spring-load hold-vs-tap, the ES:16453 auto-repeat swallow,
# the ruling-(e) announcement matrix per INPUT SOURCE, the X swap
# action, the re-anchored per-letter source pin (now over the
# dispatcher's ruled table), and a mechanical per-platform-theme
# binding sweep of every ruled sequence against the resolved
# StandardKey sets (the Alt+Backspace/Undo collision class, caught by
# machine instead of by eye).
qt_add_executable(tst_chord_table
    tests/tst_chord_table.cpp
    src/units.cpp src/units.h
    src/abi_client.cpp src/abi_client.h
    src/announcer.cpp src/announcer.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/icon_provider.cpp src/icon_provider.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/layer_thumb_provider.cpp src/layer_thumb_provider.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h
    src/tool_key_dispatcher.cpp src/tool_key_dispatcher.h
)
qt_add_qml_module(tst_chord_table
    URI AppnameShell
    VERSION 1.0
    OUTPUT_DIRECTORY probe-qml-chordtable/AppnameShell
    QML_FILES
        qml/Theme.qml
        qml/Main.qml
        qml/CanvasContextMenu.qml
        qml/ChordMenuItem.qml
        qml/ToolPalette.qml
        qml/LayersPanel.qml
        qml/HistoryPanel.qml
        qml/MaskPanel.qml
        qml/NewDocumentDialog.qml
        qml/dialogs/AdjustmentParamDialog.qml
        qml/dialogs/CurveEditor.qml
        qml/dialogs/AssignProfileDialog.qml
        qml/dialogs/RecoveryDialog.qml
        qml/dialogs/CanvasSizeDialog.qml
        qml/dialogs/ConvertProfileDialog.qml
        qml/dialogs/CropDialog.qml
        qml/dialogs/DepthConvertDialog.qml
        qml/dialogs/FillDialog.qml
        qml/dialogs/FlattenConfirmDialog.qml
        qml/dialogs/DeleteConfirmDialog.qml
        qml/dialogs/AboutDialog.qml
        qml/dialogs/LicensesDialog.qml
        qml/dialogs/DiscardConfirmDialog.qml
        qml/dialogs/ResizeDialog.qml
        qml/dialogs/RotateDialog.qml
        qml/dialogs/ModeConvertDialog.qml
        qml/dialogs/FilterParamDialog.qml
        qml/dialogs/SelectionModifyDialog.qml
        qml/dialogs/TrimDialog.qml
)
qt_add_resources(tst_chord_table "lucide_icons_chordtable"
    PREFIX "/icons/lucide"
    BASE "icons/lucide"
    FILES ${APPNAME_LUCIDE_SVGS} icons/lucide/LICENSE)
set_target_properties(tst_chord_table PROPERTIES MACOSX_BUNDLE FALSE)
target_include_directories(tst_chord_table PRIVATE
    "${APPNAME_REPO_ROOT}/include" src)
target_link_libraries(tst_chord_table PRIVATE
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Svg
    Qt6::Test "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_chord_table appname_cabi_build)
endif()
# The staged exe-adjacent i18n/ (appname-shell POST_BUILD) is this
# test's catalog source — same resolution order as the app.
add_dependencies(tst_chord_table appname-shell)
add_test(NAME tst_chord_table COMMAND tst_chord_table)

# THE IMPORT-FILTER HONESTY NET (tests/tst_import_filter.cpp). Boots the
# REAL Main.qml offscreen (its own compiled copy of the AppnameShell
# module — the tst_palette_pins harness) and pins File→Import's name
# filter against the ENGINE's accepted ingress set: the binding exists,
# the offered row is read back THROUGH the catalog (no literal, no
# key-echo, four-locale vacuity guard), and every extension the filter
# advertises is one the CP-0.5 two-door import path actually ingests —
# so swapping in the broad image row before the tiered import door lands
# fails here.
qt_add_executable(tst_import_filter
    tests/tst_import_filter.cpp
    src/units.cpp src/units.h
    src/abi_client.cpp src/abi_client.h
    src/announcer.cpp src/announcer.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/icon_provider.cpp src/icon_provider.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/layer_thumb_provider.cpp src/layer_thumb_provider.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h
)
qt_add_qml_module(tst_import_filter
    URI AppnameShell
    VERSION 1.0
    OUTPUT_DIRECTORY probe-qml-importfilter/AppnameShell
    QML_FILES
        qml/Theme.qml
        qml/Main.qml
        qml/CanvasContextMenu.qml
        qml/ChordMenuItem.qml
        qml/ToolPalette.qml
        qml/LayersPanel.qml
        qml/HistoryPanel.qml
        qml/MaskPanel.qml
        qml/NewDocumentDialog.qml
        qml/dialogs/AdjustmentParamDialog.qml
        qml/dialogs/CurveEditor.qml
        qml/dialogs/AssignProfileDialog.qml
        qml/dialogs/RecoveryDialog.qml
        qml/dialogs/CanvasSizeDialog.qml
        qml/dialogs/ConvertProfileDialog.qml
        qml/dialogs/CropDialog.qml
        qml/dialogs/DepthConvertDialog.qml
        qml/dialogs/FillDialog.qml
        qml/dialogs/FlattenConfirmDialog.qml
        qml/dialogs/DeleteConfirmDialog.qml
        qml/dialogs/AboutDialog.qml
        qml/dialogs/LicensesDialog.qml
        qml/dialogs/DiscardConfirmDialog.qml
        qml/dialogs/ResizeDialog.qml
        qml/dialogs/RotateDialog.qml
        qml/dialogs/ModeConvertDialog.qml
        qml/dialogs/FilterParamDialog.qml
        qml/dialogs/SelectionModifyDialog.qml
        qml/dialogs/TrimDialog.qml
)
qt_add_resources(tst_import_filter "lucide_icons_importfilter"
    PREFIX "/icons/lucide"
    BASE "icons/lucide"
    FILES ${APPNAME_LUCIDE_SVGS} icons/lucide/LICENSE)
set_target_properties(tst_import_filter PROPERTIES MACOSX_BUNDLE FALSE)
target_include_directories(tst_import_filter PRIVATE
    "${APPNAME_REPO_ROOT}/include" src)
target_link_libraries(tst_import_filter PRIVATE
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Svg
    Qt6::Test "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_import_filter appname_cabi_build)
endif()
# The staged exe-adjacent i18n/ (appname-shell POST_BUILD) is this
# test's catalog source — same resolution order as the app.
add_dependencies(tst_import_filter appname-shell)
add_test(NAME tst_import_filter COMMAND tst_import_filter)

# THE EXPORT-FILTER HONESTY NET (tests/tst_export_filter.cpp) — the same
# unbound-dialog defect as tst_import_filter above, one dialog over, with
# the SAVE-dialog difference: the honest set is what the encoder WRITES.
# Boots the REAL Main.qml offscreen (its own compiled copy of the
# AppnameShell module — the tst_palette_pins harness) and pins
# File→Export's rows against ENGINE TRUTH: the binding exists and is not
# Qt's substituted catch-all row, the rows are read back THROUGH the
# catalog (no literal, no key-echo, four-locale vacuity guard), and every
# extension the rows advertise maps — through the dialog's OWN handler
# helper — to a format the export door is PROVED to write by exporting
# once per built-in registry id against a real engine document. Links the
# cabi for that reason (the tst_selection_gestures registration idiom).
qt_add_executable(tst_export_filter
    tests/tst_export_filter.cpp
    src/units.cpp src/units.h
    src/abi_client.cpp src/abi_client.h
    src/announcer.cpp src/announcer.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/icon_provider.cpp src/icon_provider.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/layer_thumb_provider.cpp src/layer_thumb_provider.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h
)
qt_add_qml_module(tst_export_filter
    URI AppnameShell
    VERSION 1.0
    OUTPUT_DIRECTORY probe-qml-exportfilter/AppnameShell
    QML_FILES
        qml/Theme.qml
        qml/Main.qml
        qml/CanvasContextMenu.qml
        qml/ChordMenuItem.qml
        qml/ToolPalette.qml
        qml/LayersPanel.qml
        qml/HistoryPanel.qml
        qml/MaskPanel.qml
        qml/NewDocumentDialog.qml
        qml/dialogs/AdjustmentParamDialog.qml
        qml/dialogs/CurveEditor.qml
        qml/dialogs/AssignProfileDialog.qml
        qml/dialogs/RecoveryDialog.qml
        qml/dialogs/CanvasSizeDialog.qml
        qml/dialogs/ConvertProfileDialog.qml
        qml/dialogs/CropDialog.qml
        qml/dialogs/DepthConvertDialog.qml
        qml/dialogs/FillDialog.qml
        qml/dialogs/FlattenConfirmDialog.qml
        qml/dialogs/DeleteConfirmDialog.qml
        qml/dialogs/AboutDialog.qml
        qml/dialogs/LicensesDialog.qml
        qml/dialogs/DiscardConfirmDialog.qml
        qml/dialogs/ResizeDialog.qml
        qml/dialogs/RotateDialog.qml
        qml/dialogs/ModeConvertDialog.qml
        qml/dialogs/FilterParamDialog.qml
        qml/dialogs/SelectionModifyDialog.qml
        qml/dialogs/TrimDialog.qml
)
qt_add_resources(tst_export_filter "lucide_icons_exportfilter"
    PREFIX "/icons/lucide"
    BASE "icons/lucide"
    FILES ${APPNAME_LUCIDE_SVGS} icons/lucide/LICENSE)
set_target_properties(tst_export_filter PROPERTIES MACOSX_BUNDLE FALSE)
target_include_directories(tst_export_filter PRIVATE
    "${APPNAME_REPO_ROOT}/include" src)
target_link_libraries(tst_export_filter PRIVATE
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Svg
    Qt6::Test "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_export_filter appname_cabi_build)
endif()
# The staged exe-adjacent i18n/ (appname-shell POST_BUILD) is this
# test's catalog source — same resolution order as the app.
add_dependencies(tst_export_filter appname-shell)
add_test(NAME tst_export_filter COMMAND tst_export_filter)

# PERF PROBE HARNESS (tests/tst_perf_probes.cpp) — bench-class, runnable
# ON DEMAND, and NOT part of the default green suite (wall-clock numbers
# would flake CI). The target always BUILDS; ctest registration is opt-in:
#   cmake ... -DAPPNAME_BENCH_CTEST=ON     # registers with LABELS "bench"
#   ctest --test-dir <build> -L bench      # run it
#   ctest --test-dir <build> -LE bench     # default suite even when ON
# With the option OFF (default) a plain `ctest` run is byte-identical to
# pre-harness: the default test count is unchanged. Direct invocation
# (no ctest): <build>/tst_perf_probes — see the file header for the
# measurement discipline (warm reps, min/median, A/B/A pairing).
option(APPNAME_BENCH_CTEST
    "Register tst_perf_probes with ctest (LABELS bench; excluded via -LE bench)"
    OFF)
qt_add_executable(tst_perf_probes
    tests/tst_perf_probes.cpp
    src/abi_client.cpp src/abi_client.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/announcer.cpp src/announcer.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h)
set_target_properties(tst_perf_probes PROPERTIES MACOSX_BUNDLE FALSE)
target_include_directories(tst_perf_probes PRIVATE
    "${APPNAME_REPO_ROOT}/include" src)
# The probe output names the config it measured (Release is the
# reportable one; Debug numbers are triage-only).
target_compile_definitions(tst_perf_probes PRIVATE
    APPNAME_PROBE_BUILD_CONFIG="$<IF:$<CONFIG:Release>,Release,$<CONFIG>>")
target_link_libraries(tst_perf_probes PRIVATE
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::Test "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_perf_probes appname_cabi_build)
endif()
if(APPNAME_BENCH_CTEST)
    add_test(NAME tst_perf_probes COMMAND tst_perf_probes)
    set_tests_properties(tst_perf_probes PROPERTIES LABELS "bench")
endif()

# G-5 REPRO + COMMIT-PATH FRAME PROBE (tests/tst_g5_frameprobe.cpp) —
# THE EXCLUSION IS PER-CASE, NOT PER-TARGET. Re-scoped by operator ruling
# 2026-08-26 after half its original rationale expired.
#
# It previously read "NEVER registered with ctest", for two stated reasons:
#   (a) wall-clock numbers would flake the default suite   — STILL TRUE
#   (b) the G-5 case is a REGRESSION REPRODUCTION that reds by design
#       "UNTIL THE REPAINT SCOPING IS CORRECTED"           — ★ EXPIRED
# The repaint scoping was corrected and merged at 907ee4c5. Reason (b) named
# its own expiry condition and the condition was met, so the blanket exclusion
# outlived the thing it protected against — and nothing in the tree noticed,
# because PROSE DOES NOT EXPIRE LOUDLY. That cost real coverage: the only test
# that can observe the repaint invalidation rect sat unregistered, and
# `ctest -R g5_frameprobe` answered "No tests were found!!!" AND EXITED 0
# (battery_guards.sh:183), so "execute the Qt suites" would not have run it.
#
# REMAINING EXCLUSION, with its own expiry condition stated (G-124):
#   commitPathFrameLatency2048  — asserts WALL-CLOCK; excluded from the default
#     suite because timing flakes it. EXPIRES if the case stops asserting
#     latency, or if a tier exists that tolerates timing variance.
#   g5ShapeCommitRendersCheckerboardOverImage — a regression REPRODUCTION.
#     EXPIRES the moment it is expected to pass; it is a live red by design
#     only while G-5 reproduces. RE-CHECK THIS ONE: (b) already expired once.
# REGISTERED (f3c94410): transformDragRepaintFootprint asserts counts and
#   pixels only — paints>0, zero stale pixels against a full repaint, a colour
#   compare at a vacated point — no latency assertion, ~6 s. Neither remaining
#   reason covers it.
#
# Run the excluded cases directly:
#   <build>/tst_g5_frameprobe g5ShapeCommitRendersCheckerboardOverImage
#   <build>/tst_g5_frameprobe commitPathFrameLatency2048
# The file header carries the instrument rule: an UNRENDERED leg is a
# hard failure, never a fast number.
qt_add_executable(tst_g5_frameprobe
    tests/tst_g5_frameprobe.cpp
    src/abi_client.cpp src/abi_client.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/announcer.cpp src/announcer.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h)
set_target_properties(tst_g5_frameprobe PROPERTIES MACOSX_BUNDLE FALSE)
target_include_directories(tst_g5_frameprobe PRIVATE
    "${APPNAME_REPO_ROOT}/include" src)
target_compile_definitions(tst_g5_frameprobe PRIVATE
    APPNAME_PROBE_BUILD_CONFIG="$<IF:$<CONFIG:Release>,Release,$<CONFIG>>")
target_link_libraries(tst_g5_frameprobe PRIVATE
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::Test "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_g5_frameprobe appname_cabi_build)
endif()

# C-353: ONE CASE OF THAT FILE IS REGISTERED, AND ONLY ONE.
# The wholesale exclusion above is correct for the two cases it was written
# for -- `commitPathFrameLatency2048` reports wall-clock and would flake the
# default suite, and `g5Shape…` is a regression reproduction that reds by
# design. Neither rationale covers `transformDragRepaintFootprint`, added with
# item 21's scoped update(rect): it asserts COUNTS and PIXELS only -- paints>0,
# zero stale pixels against a full repaint, and a colour compare at a vacated
# point -- with no latency assertion anywhere, and it runs in ~6 s.
#
# WHY THIS MATTERS BEYOND TIDINESS: it is the ONLY test that can observe the
# repaint invalidation rect. Measured at C-353, `ctest -R g5_frameprobe`
# answered "No tests were found!!!" AND EXITED 0 -- the trap battery_guards.sh
# documents at :183 -- so "execute the Qt suites" would not have run it and the
# render-behaviour carve-out would have persisted for a reason nobody had
# stated. Registering the deterministic case, and nothing else, closes that.
add_test(NAME tst_transform_repaint_footprint
         COMMAND tst_g5_frameprobe transformDragRepaintFootprint)

# q-gesture (standing QA session): the CHROME half of the gesture net —
# boots the REAL Main.qml (the tst_palette_pins harness pattern) and joins
# the two halves nobody joins: pick a tool through the shipping chrome
# (palette slot click / single-key letter), then COMPLETE that tool's real
# gesture and assert engine truth.
qt_add_executable(tst_gesture_chrome
    tests/tst_gesture_chrome.cpp
    src/abi_client.cpp src/abi_client.h
    src/announcer.cpp src/announcer.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/icon_provider.cpp src/icon_provider.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/layer_thumb_provider.cpp src/layer_thumb_provider.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h
    src/tool_key_dispatcher.cpp src/tool_key_dispatcher.h
    src/units.cpp src/units.h
)
# q-gesture: the QML file list is GLOBBED, not hand-maintained. It has
# drifted twice now (T4 added three files by hand; a later rebase silently
# dropped DeleteConfirmDialog.qml, and Main.qml then failed to load at
# T14). CONFIGURE_DEPENDS re-globs when the set changes, so this target
# tracks the app's QML automatically instead of decaying every wave.
file(GLOB_RECURSE APPNAME_QGESTURE_QML CONFIGURE_DEPENDS
    RELATIVE "${CMAKE_CURRENT_SOURCE_DIR}"
    "${CMAKE_CURRENT_SOURCE_DIR}/qml/*.qml")
qt_add_qml_module(tst_gesture_chrome
    URI AppnameShell
    VERSION 1.0
    OUTPUT_DIRECTORY qgesture-qml/AppnameShell
    QML_FILES ${APPNAME_QGESTURE_QML}
)
qt_add_resources(tst_gesture_chrome "lucide_icons_qgesture"
    PREFIX "/icons/lucide"
    BASE "icons/lucide"
    FILES ${APPNAME_LUCIDE_SVGS} icons/lucide/LICENSE)
set_target_properties(tst_gesture_chrome PROPERTIES MACOSX_BUNDLE FALSE)
target_include_directories(tst_gesture_chrome PRIVATE
    "${APPNAME_REPO_ROOT}/include" src)
target_compile_definitions(tst_gesture_chrome PRIVATE
    APPNAME_REPO_I18N_DIR="${APPNAME_REPO_ROOT}/i18n")
target_link_libraries(tst_gesture_chrome PRIVATE
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Svg
    Qt6::Test "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_gesture_chrome appname_cabi_build)
endif()
add_dependencies(tst_gesture_chrome appname-shell)
add_test(NAME tst_gesture_chrome COMMAND tst_gesture_chrome)

# q-gesture (standing QA session): the GROWING half of the gesture net.
target_sources(tst_gesture_net PRIVATE
    src/abi_client.cpp src/abi_client.h
    src/canvas_view.cpp src/canvas_view.h
    src/clipboard_ops.cpp src/clipboard_ops.h
    src/document_controller.cpp src/document_controller.h
    src/envelope_builder.cpp src/envelope_builder.h
    src/font_meta.cpp src/font_meta.h
    src/i18n_catalog.cpp src/i18n_catalog.h
    src/announcer.cpp src/announcer.h
    src/sample_blob.cpp src/sample_blob.h
    src/selection_scan.cpp src/selection_scan.h)
target_include_directories(tst_gesture_net PRIVATE
    "${APPNAME_REPO_ROOT}/include")
target_compile_definitions(tst_gesture_net PRIVATE
    APPNAME_TOOLPALETTE_QML="${CMAKE_CURRENT_SOURCE_DIR}/qml/ToolPalette.qml"
    APPNAME_REPO_REPORTS="${APPNAME_REPO_ROOT}/reports"
    APPNAME_MAIN_QML="${CMAKE_CURRENT_SOURCE_DIR}/qml/Main.qml"
    APPNAME_REPO_ROOT="${APPNAME_REPO_ROOT}")
target_link_libraries(tst_gesture_net PRIVATE
    Qt6::Qml Qt6::Quick "${APPNAME_CABI_LIB}")
if(APPNAME_CABI_BUILD)
    add_dependencies(tst_gesture_net appname_cabi_build)
endif()
