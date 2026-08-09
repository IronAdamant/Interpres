# Build a Windows-only portable release pack (no macOS app, no crates.io deps).
# Usage (from repo root or this folder):
#   powershell -NoProfile -ExecutionPolicy Bypass -File packaging\make-windows-release.ps1
#
# Requires: Rust (stable), and either:
#   - x86_64-pc-windows-msvc + Visual C++ Build Tools, or
#   - x86_64-pc-windows-gnu + MinGW on PATH

$ErrorActionPreference = 'Stop'

$Root = Resolve-Path (Join-Path $PSScriptRoot '..')
Set-Location $Root

Write-Host '==> Building Windows release binary...'
# Prefer GNU/binutils windres (WinLibs). LLVM-MinGW windres + GNU ld often
# links a broken .rsrc tree so Explorer shows the default exe icon.
$preferWindres = @(
    "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin\windres.exe"
)
# Also accept any non-LLVM windres already on PATH
Get-Command windres -All -ErrorAction SilentlyContinue | ForEach-Object {
    $preferWindres += $_.Source
}
if (-not $env:WINDRES) {
    foreach ($c in $preferWindres) {
        if (-not $c -or -not (Test-Path $c)) { continue }
        if ($c -match 'LLVM-MinGW|llvm-mingw') { continue }
        $env:WINDRES = $c
        $binDir = Split-Path $c -Parent
        # Put matching MinGW bin first so the linker matches windres (GNU ld).
        $env:Path = "$binDir;$env:Path"
        break
    }
}
if ($env:WINDRES) {
    Write-Host "    windres: $env:WINDRES"
} else {
    Write-Host "    warning: no GNU windres found - exe may lack app icon"
}
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed ($LASTEXITCODE)" }

$ExeSrc = Join-Path $Root 'target\release\interpres.exe'
if (-not (Test-Path $ExeSrc)) {
    throw "Missing $ExeSrc - build did not produce interpres.exe"
}

$Dist = Join-Path $Root 'dist\Interpres-windows'
if (Test-Path $Dist) { Remove-Item -Recurse -Force $Dist }
New-Item -ItemType Directory -Path (Join-Path $Dist 'helpers\windows') | Out-Null

Copy-Item $ExeSrc (Join-Path $Dist 'interpres.exe')
Copy-Item (Join-Path $Root 'helpers\windows\Get-LiveCaptionsText.ps1') (Join-Path $Dist 'helpers\windows\Get-LiveCaptionsText.ps1')
# Also put helper next to the exe for simplest discovery
Copy-Item (Join-Path $Root 'helpers\windows\Get-LiveCaptionsText.ps1') (Join-Path $Dist 'Get-LiveCaptionsText.ps1')
Copy-Item (Join-Path $Root 'README.md') (Join-Path $Dist 'README.md')
# Windows pack branding: cleaned symbol (logo-256). Mac uses the same mark via
# logo.png / logo-1024.png / Interpres.icns (regenerated from logo-256).
if (Test-Path (Join-Path $Root 'assets\logo-256.png')) {
    Copy-Item (Join-Path $Root 'assets\logo-256.png') (Join-Path $Dist 'logo-256.png')
    Copy-Item (Join-Path $Root 'assets\logo-256.png') (Join-Path $Dist 'logo.png')
} elseif (Test-Path (Join-Path $Root 'assets\logo.png')) {
    Copy-Item (Join-Path $Root 'assets\logo.png') (Join-Path $Dist 'logo.png')
}
if (Test-Path (Join-Path $Root 'assets\Interpres.ico')) {
    Copy-Item (Join-Path $Root 'assets\Interpres.ico') (Join-Path $Dist 'Interpres.ico')
}

$openBat = @'
@echo off
cd /d "%~dp0"
if not exist interpres.exe (
  echo interpres.exe not found in this folder.
  pause
  exit /b 1
)
rem Native Win32 window (same core as CLI; no install)
start "" interpres.exe
'@
Set-Content -Path (Join-Path $Dist 'Open Interpres.bat') -Value $openBat -Encoding ASCII

$demoBat = @'
@echo off
cd /d "%~dp0"
interpres.exe demo
pause
'@
Set-Content -Path (Join-Path $Dist 'Try demo.bat') -Value $demoBat -Encoding ASCII

$probeBat = @'
@echo off
cd /d "%~dp0"
interpres.exe probe
echo.
pause
'@
Set-Content -Path (Join-Path $Dist 'Check Live Captions (probe).bat') -Value $probeBat -Encoding ASCII

$diagBat = @'
@echo off
cd /d "%~dp0"
interpres.exe diagnose
echo.
pause
'@
Set-Content -Path (Join-Path $Dist 'Diagnose.bat') -Value $diagBat -Encoding ASCII

$onBat = @'
@echo off
cd /d "%~dp0"
interpres.exe remember on
echo.
pause
'@
Set-Content -Path (Join-Path $Dist 'Turn saving ON.bat') -Value $onBat -Encoding ASCII

$offBat = @'
@echo off
cd /d "%~dp0"
interpres.exe remember off
echo.
pause
'@
Set-Content -Path (Join-Path $Dist 'Turn saving OFF.bat') -Value $offBat -Encoding ASCII

$startHere = @"
Interpres - Windows portable pack

WHAT THIS IS
  Free helper that can SAVE Windows Live Captions as text files.
  Not a captioner by itself. Everything stays on your PC.

EASY START (no tech skills needed)
  1. Turn on Live Captions:  Win + Ctrl + L
  2. Play audio so captions appear on screen
  3. Double-click  interpres.exe  (or Open Interpres.bat)
  4. Press Start listening
  5. Optional: Save to disk ON, Choose folder
     Default folder: Documents\Interpres Transcripts

TIPS
  - You should NOT see terminal windows pop up while listening.
  - Get-LiveCaptionsText.ps1 is included (and the app can recreate it if missing).
  - Wrong words in the file usually mean Live Captions misheard them.

ALSO TRY
  Try demo.bat                     sample text without Live Captions
  Check Live Captions (probe).bat  is Live Captions running?
  Diagnose.bat                     more detail if capture fails

ADVANCED (optional terminal)
  interpres.exe probe
  interpres.exe diagnose
  interpres.exe remember on
  interpres.exe set-folder "D:\My Captions"
"@
Set-Content -Path (Join-Path $Dist 'START HERE.txt') -Value $startHere -Encoding UTF8

# Checksums + source stamp
$Hash = (Get-FileHash -Algorithm SHA256 $ExeSrc).Hash.ToLowerInvariant()
Set-Content -Path (Join-Path $Dist 'SHA256SUMS.txt') -Value "SHA256  interpres.exe`r`n$Hash" -Encoding ASCII

$commit = ''
try {
    $commit = (git -C $Root rev-parse HEAD 2>$null).Trim()
} catch { }
if (-not $commit) { $commit = 'unknown' }
Set-Content -Path (Join-Path $Dist 'SOURCE_COMMIT.txt') -Value $commit -Encoding ASCII
Set-Content -Path (Join-Path $Root 'dist\SOURCE_COMMIT-windows.txt') -Value $commit -Encoding ASCII
Set-Content -Path (Join-Path $Root 'dist\SHA256SUMS-windows.txt') -Value "SHA256  interpres.exe`r`n$Hash" -Encoding ASCII

# Zip
$Zip = Join-Path $Root 'dist\Interpres-portable-windows.zip'
if (Test-Path $Zip) { Remove-Item -Force $Zip }
Compress-Archive -Path $Dist -DestinationPath $Zip -Force

Write-Host ''
Write-Host 'Done.'
Write-Host "  Folder: $Dist"
Write-Host "  Zip:    $Zip"
Write-Host "  SHA256: $Hash"
Write-Host "  COMMIT: $commit"
Write-Host ''
Write-Host 'Double-click Open Interpres.bat in the folder after Live Captions is on.'
