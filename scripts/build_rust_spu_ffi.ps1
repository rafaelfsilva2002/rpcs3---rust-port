# R6.1 — Build the rpcs3-spu-ffi staticlib that the C++ bridge links.
# Windows / PowerShell variant of build_rust_spu_ffi.sh — same contract.
#
# Usage:
#   pwsh scripts\build_rust_spu_ffi.ps1
#   pwsh scripts\build_rust_spu_ffi.ps1 -Debug
#   pwsh scripts\build_rust_spu_ffi.ps1 -DestRoot R:\
#   $env:RPCS3_RUST_DEST_ROOT = 'R:\'; pwsh scripts\build_rust_spu_ffi.ps1
#
# Outputs (under repo root):
#   rust\target\release\rpcs3_spu_ffi.lib       (or target\debug\)
#   rust\rpcs3-spu-ffi\include\rpcs3_spu_ffi.h  (cbindgen-generated)
#
# When -DestRoot is supplied (or $env:RPCS3_RUST_DEST_ROOT is set),
# the .lib + header are also copied into:
#   <DestRoot>\rust\target\<mode>\rpcs3_spu_ffi.lib
#   <DestRoot>\rust\rpcs3-spu-ffi\include\rpcs3_spu_ffi.h
# (R6.1c — added so the MSVC EXISTS-conditional in emucore.vcxproj
#  resolves on the build tree without manual copying.)
#
# Exit codes:
#   0 — staticlib + header produced (and copied if requested).
#   1 — cargo build failed.
#   2 — cbindgen missing.
#   3 — header generation failed.
#   4 — DestRoot copy failed.

param(
    [switch]$Debug,
    [string]$DestRoot
)

$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$RustDir  = Join-Path $RepoRoot "rust"
$FfiDir   = Join-Path $RustDir "rpcs3-spu-ffi"

$Mode = if ($Debug) { "debug" } else { "release" }
Write-Host "[build_rust_spu_ffi] mode=$Mode"

Push-Location $RustDir
try {
    if ($Mode -eq "release") {
        cargo build --release -p rpcs3-spu-ffi
    } else {
        cargo build -p rpcs3-spu-ffi
    }
    if ($LASTEXITCODE -ne 0) { exit 1 }
} finally {
    Pop-Location
}

Write-Host "[build_rust_spu_ffi] regenerating header via cbindgen..."
$cbindgen = Get-Command cbindgen -ErrorAction SilentlyContinue
if ($null -eq $cbindgen) {
    Write-Error "cbindgen not found in PATH; install via 'cargo install cbindgen'"
    exit 2
}

Push-Location $FfiDir
try {
    cbindgen --config cbindgen.toml --crate rpcs3-spu-ffi --output (Join-Path "include" "rpcs3_spu_ffi.h")
    if ($LASTEXITCODE -ne 0) { exit 3 }
} finally {
    Pop-Location
}

$libPath = Join-Path $RustDir "target\$Mode\rpcs3_spu_ffi.lib"
$hdrPath = Join-Path $FfiDir "include\rpcs3_spu_ffi.h"
Write-Host "[build_rust_spu_ffi] OK"
Write-Host "[build_rust_spu_ffi]   staticlib: $libPath"
Write-Host "[build_rust_spu_ffi]   header:    $hdrPath"

# R6.1c — optional auto-copy to a destination rpcs3 source tree
# (e.g. R:\) so MSVC's Exists() condition in emucore.vcxproj /
# rpcs3.vcxproj picks up the staticlib without manual copying.
if (-not $DestRoot -and $env:RPCS3_RUST_DEST_ROOT) {
    $DestRoot = $env:RPCS3_RUST_DEST_ROOT
    Write-Host "[build_rust_spu_ffi] DestRoot from `$env:RPCS3_RUST_DEST_ROOT = $DestRoot"
}

if ($DestRoot) {
    if (-not (Test-Path $DestRoot -PathType Container)) {
        Write-Error "[build_rust_spu_ffi] DestRoot '$DestRoot' does not exist or is not a directory"
        exit 4
    }
    $DestLibDir = Join-Path $DestRoot "rust\target\$Mode"
    $DestHdrDir = Join-Path $DestRoot "rust\rpcs3-spu-ffi\include"
    try {
        New-Item -ItemType Directory -Force -Path $DestLibDir | Out-Null
        New-Item -ItemType Directory -Force -Path $DestHdrDir | Out-Null
        Copy-Item -Force $libPath (Join-Path $DestLibDir "rpcs3_spu_ffi.lib")
        Copy-Item -Force $hdrPath (Join-Path $DestHdrDir "rpcs3_spu_ffi.h")
        # Also copy debug symbols if present (helps MSVC linker locate PDB).
        $pdbPath = Join-Path $RustDir "target\$Mode\rpcs3_spu_ffi.pdb"
        if (Test-Path $pdbPath) {
            Copy-Item -Force $pdbPath (Join-Path $DestLibDir "rpcs3_spu_ffi.pdb")
        }
        Write-Host "[build_rust_spu_ffi] copied to DestRoot:"
        Write-Host "[build_rust_spu_ffi]   $(Join-Path $DestLibDir 'rpcs3_spu_ffi.lib')"
        Write-Host "[build_rust_spu_ffi]   $(Join-Path $DestHdrDir 'rpcs3_spu_ffi.h')"
    } catch {
        Write-Error "[build_rust_spu_ffi] copy to DestRoot failed: $_"
        exit 4
    }
} else {
    Write-Host "[build_rust_spu_ffi] (no -DestRoot supplied; staticlib stays in $RustDir)"
}

Write-Host ""
Write-Host "Next: re-run cmake configure or the MSBuild bridge.props import"
Write-Host "to pick up the staticlib. Bridge stays runtime-gated by"
Write-Host "RPCS3_SPU_RUST_BRIDGE=1 (default OFF)."
