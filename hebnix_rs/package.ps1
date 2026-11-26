<#
build release + assemble a clean dist/ with only what ships. nothing from
target/ (no deps/, build/, .ilk, .d...).

dist/ has 4 variant folders, all from the same 2 exes:
  hebnix / hebnix-lite                     standard, data in %AppData%\Hebnix
  hebnix-portable / hebnix-lite-portable   portable, data in the exe folder

portable drops a portable.txt next to the exe, which flips the app's base dir
to the exe folder (the pre 2.1.8 layout). standard has no marker.

The four folders contain the application executable and optional portable
marker/PDB. Other bundled assets are extracted by the app when needed.

pdb (target/release/hebnix.pdb, big) is not shipped by default. keep it
archived per release so you can symbolicate a user's crash.txt later. -WithPdb
bundles it with the full variants instead (readable crashes, way bigger dl).
#>
param([switch]$WithPdb)

$ErrorActionPreference = 'Stop'
$root       = $PSScriptRoot                       # hebnix_rs
$releaseDir = Join-Path $root 'target\release'
$distDir    = Join-Path $root 'dist'

Write-Host 'building release...' -ForegroundColor Cyan
Push-Location $root
try {
    Get-Process hebnix -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    # cargo writes progress to stderr, which under ErrorActionPreference=Stop
    # would abort the script even on success. relax it for the build, then
    # check the real exit code.
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    cargo build --locked --release --bin hebnix
    $code = $LASTEXITCODE
    $ErrorActionPreference = $prev
    if ($code -ne 0) { throw "cargo build failed ($code)" }
    $ErrorActionPreference = 'Continue'
    cargo build --locked --release --bin hebnix-lite --features lite
    $code = $LASTEXITCODE
    $ErrorActionPreference = $prev
    if ($code -ne 0) { throw "lite build failed ($code)" }
} finally {
    Pop-Location
}

Write-Host 'assembling dist/...' -ForegroundColor Cyan
if (Test-Path $distDir) { Remove-Item $distDir -Recurse -Force }
New-Item -ItemType Directory -Path $distDir | Out-Null

$variants = @(
    @{ Name = 'hebnix';               Exe = 'hebnix.exe';      Portable = $false; Pdb = $true  },
    @{ Name = 'hebnix-lite';          Exe = 'hebnix-lite.exe'; Portable = $false; Pdb = $false },
    @{ Name = 'hebnix-portable';      Exe = 'hebnix.exe';      Portable = $true;  Pdb = $true  },
    @{ Name = 'hebnix-lite-portable'; Exe = 'hebnix-lite.exe'; Portable = $true;  Pdb = $false }
)

foreach ($v in $variants) {
    $outDir = Join-Path $distDir $v.Name
    New-Item -ItemType Directory -Path $outDir | Out-Null

    $files = @((Join-Path $releaseDir $v.Exe))
    if ($WithPdb -and $v.Pdb) { $files += (Join-Path $releaseDir 'hebnix.pdb') }

    foreach ($f in $files) {
        if (-not (Test-Path $f)) { throw "build output missing: $f" }
        $destination = if ($v.Exe -eq 'hebnix-lite.exe') { Join-Path $outDir 'Hebnix Lite.exe' } else { $outDir }
        Copy-Item -LiteralPath $f -Destination $destination
    }
    if ($v.Portable) {
        Set-Content -Path (Join-Path $outDir 'portable.txt') -Value 'keep hebnix data in this folder instead of %AppData%\Hebnix' -Encoding utf8
    }
    Write-Host "  + $($v.Name)"
}

Write-Host "`ndist/ ready: $distDir" -ForegroundColor Green
Get-ChildItem $distDir -Directory | ForEach-Object {
    Write-Host $_.Name -ForegroundColor White
    Get-ChildItem $_.FullName | Select-Object Name, @{N='Size';E={"{0:N0} KB" -f ($_.Length/1KB)}} | Format-Table -AutoSize
}

if (-not $WithPdb) {
    $pdb = Join-Path $releaseDir 'hebnix.pdb'
    Write-Host "no pdb shipped. archive this per release to read crash.txt later:" -ForegroundColor Yellow
    Write-Host "  $pdb"
}

