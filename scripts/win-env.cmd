@echo off
rem SPDX-License-Identifier: GPL-3.0-only
rem Copyright (c) 2026 the Appname contributors
rem
rem Windows build environment (O-WIN, WS-D). Usage, from any prompt:
rem     scripts\win-env.cmd <command...>
rem e.g. scripts\win-env.cmd cargo build --release --workspace
rem
rem 1. Enters the VS 2022 x64 toolchain (vcvars64), so cc/cmake crates find
rem    MSVC and MSVC's link.exe precedes Git-Bash's coreutils `link`.
rem 2. W-03: freetype-sys 0.23 (freetype-rs `bundled`) compiles its vendored
rem    libpng with an include path that is absent from the published crate;
rem    macOS takes zlib.h from the SDK, MSVC has none. Point cc at the zlib
rem    headers libz-sys already vendors, located via `cargo metadata` (no
rem    machine path is baked in). MSVC target only.
setlocal
rem W-26: a session started from another VS's developer prompt (e.g. VS 2026,
rem VSCMD_VER 18.x) leaves its INCLUDE/LIB first; vcvars64 below then mixes
rem 2022's cl with that STL (STL1001). Drop the inherited dev variables; PATH
rem is kept (vcvars64 puts 2022's bin first; later entries such as bash stay).
for %%v in (INCLUDE LIB LIBPATH EXTERNAL_INCLUDE VSCMD_VER VSCMD_ARG_app_plat VSCMD_ARG_HOST_ARCH VSCMD_ARG_TGT_ARCH VSINSTALLDIR VCINSTALLDIR VCToolsInstallDir VCToolsVersion VCToolsRedistDir VCIDEInstallDir DevEnvDir WindowsSdkDir WindowsSdkBinPath WindowsSdkVerBinPath WindowsLibPath WindowsSDKLibVersion WindowsSDKVersion UCRTVersion UniversalCRTSdkDir ExtensionSdkDir Platform CommandPromptType __VSCMD_PREINIT_PATH __DOTNET_PREFERRED_BITNESS __DOTNET_ADD_64BIT) do set "%%v="
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not defined APPNAME_VSDIR (
    for /f "usebackq delims=" %%i in (`"%VSWHERE%" -version [17.0^,18.0^) -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "APPNAME_VSDIR=%%i"
)
if not defined APPNAME_VSDIR (
    echo win-env: no VS 2022 installation with the x64 C++ tools found 1>&2
    exit /b 2
)
set "PATH=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer;%PATH%"
call "%APPNAME_VSDIR%\VC\Auxiliary\Build\vcvars64.bat" >nul || exit /b 2

pushd "%~dp0.."
for /f "usebackq delims=" %%z in (`python -c "import json,subprocess,os;m=json.loads(subprocess.check_output(['cargo','metadata','--format-version','1','--locked']));print(os.path.join(os.path.dirname([p for p in m['packages'] if p['name']=='libz-sys'][0]['manifest_path']),'src','zlib'))"`) do set "APPNAME_ZLIB_INC=%%z"
popd
if not exist "%APPNAME_ZLIB_INC%\zlib.h" (
    echo win-env: libz-sys zlib headers not found ^(W-03^) 1>&2
    exit /b 2
)
set "CFLAGS_x86_64_pc_windows_msvc=/I%APPNAME_ZLIB_INC% %CFLAGS_x86_64_pc_windows_msvc%"
rem W-13: Windows Python writes stdout in the console code page (cp1252), so the
rem gate's Python steps crash printing catalog text (i18n/tools/audit.py on the
rem pseudo locale). UTF-8 mode, as on macOS.
set "PYTHONUTF8=1"
rem W-16: the Qt test binaries are GUI-subsystem executables; with no console
rem attached Qt logs to OutputDebugString, so ctest and redirected runs capture
rem nothing. Force Qt logging to stderr.
set "QT_FORCE_STDERR_LOGGING=1"

%*
exit /b %ERRORLEVEL%
