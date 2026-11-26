@echo off
REM Hebnix Windows Build Script (Rust only)
setlocal enabledelayedexpansion
set "HEBNIX_RS_DIR=%~dp0hebnix_rs"
set "TARGET=build"

:parse_args
if "%~1"=="" goto build
if /I "%~1"=="--help" goto show_help
if /I "%~1"=="-h" goto show_help
if /I "%~1"=="clean" (
    set "TARGET=clean"
    shift
    goto parse_args
)
if /I "%~1"=="build" (
    set "TARGET=build"
    shift
    goto parse_args
)
if /I "%~1"=="package" (
    set "TARGET=package"
    shift
    goto parse_args
)
REM Retained for existing callers; no external dependencies need installation.
if /I "%~1"=="--skip-deps" (
    shift
    goto parse_args
)
echo Unknown argument: %~1
exit /b 1

:build
if "%TARGET%"=="clean" goto do_clean
where cargo >nul 2>&1
if errorlevel 1 (
    echo ERROR: Rust not found. Install from https://rustup.rs
    exit /b 1
)
pushd "%HEBNIX_RS_DIR%"
if "%TARGET%"=="package" (
    powershell -NoProfile -ExecutionPolicy Bypass -File package.ps1
    set "BUILD_ERR=!errorlevel!"
    popd
    exit /b !BUILD_ERR!
)
echo Building hebnix-app (release)...
cargo build --locked --release --bin hebnix
if errorlevel 1 (
    popd
    exit /b 1
)
echo Building hebnix-lite (release)...
cargo build --locked --release --bin hebnix-lite --features lite
set "BUILD_ERR=!errorlevel!"
popd
if !BUILD_ERR! neq 0 exit /b !BUILD_ERR!
echo Build complete: %HEBNIX_RS_DIR%\target\release\
exit /b 0

:do_clean
pushd "%HEBNIX_RS_DIR%"
cargo clean
set "BUILD_ERR=!errorlevel!"
popd
if !BUILD_ERR! neq 0 exit /b !BUILD_ERR!
powershell -NoProfile -Command "$target = [IO.Path]::GetFullPath('%HEBNIX_RS_DIR%\dist'); $root = [IO.Path]::GetFullPath('%HEBNIX_RS_DIR%'); if (-not $target.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Clean target outside workspace' }; if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target -Recurse -Force }"
exit /b %errorlevel%

:show_help
echo Usage: build.bat [build^|package^|clean]
echo   build         Build Rust components (default)
echo   package       Build and create distribution package
echo   clean         Remove build artifacts
echo   -h, --help    Show this help message
exit /b 0
