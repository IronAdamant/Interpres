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
    Write-Host "    warning: no GNU windres found — exe may lack app icon"
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
if (Test-Path (Join-Path $Root 'assets\logo.png')) {
    Copy-Item (Join-Path $Root 'assets\logo.png') (Join-Path $Dist 'logo.png')
}
if (Test-Path (Join-Path $Root 'assets\Interpres.ico')) {
    Copy-Item (Join-Path $Root 'assets\Interpres.ico') (Join-Path $Dist 'Interpres.ico')
}
if (Test-Path (Join-Path $Root 'assets\logo-256.png')) {
    Copy-Item (Join-Path $Root 'assets\logo-256.png') (Join-Path $Dist 'logo-256.png')
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
  A free helper that can save Windows Live Captions as text files.
  Open source (MIT OR Apache-2.0). Everything stays on your PC.

EASY START
  1. Turn on Live Captions:  Win + Ctrl + L
  2. Double-click  interpres.exe  (or Open Interpres.bat)
     -> a normal Windows window opens (buttons + live text)
  3. Press Start listening; optional Save to disk / Choose folder
     Files go to: Documents\Interpres Transcripts

ALSO TRY
  Try demo.bat                          sample transcript (no Live Captions)
  Check Live Captions (probe).bat       is Live Captions running?
  Diagnose.bat                          helper / window details

IMPORTANT
  Keep Get-LiveCaptionsText.ps1 in this folder (already included).
  Interpres reads the Live Captions window via UI Automation - it is not
  a cloud speech service.

ADVANCED (terminal)
  interpres.exe run
  interpres.exe probe
  interpres.exe diagnose
  interpres.exe remember on
  interpres.exe set-folder "D:\My Captions"

BUILD
  cargo build --release
  packaging\make-windows-release.ps1
"@
Set-Content -Path (Join-Path $Dist 'START HERE.txt') -Value $startHere -Encoding UTF8

# Checksums
$Hash = (Get-FileHash -Algorithm SHA256 $ExeSrc).Hash.ToLowerInvariant()
Set-Content -Path (Join-Path $Dist 'SHA256SUMS.txt') -Value "SHA256  interpres.exe`r`n$Hash" -Encoding ASCII

# Zip
$Zip = Join-Path $Root 'dist\Interpres-portable-windows.zip'
if (Test-Path $Zip) { Remove-Item -Force $Zip }
Compress-Archive -Path $Dist -DestinationPath $Zip -Force

Write-Host ''
Write-Host 'Done.'
Write-Host "  Folder: $Dist"
Write-Host "  Zip:    $Zip"
Write-Host "  SHA256: $Hash"
Write-Host ''
Write-Host 'Double-click Open Interpres.bat in the folder after Live Captions is on.'
