# Windows PowerShell 5.1+; run from any directory.
[CmdletBinding()]
param(
    [ValidateSet('x64', 'arm64', 'all')][string]$Architecture = 'all',
    [switch]$CheckOnly,
    [string]$BaseApk = '',
    [string]$VcpkgRoot = '',
    [string]$Version = '1.1.0'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$Root = Split-Path -Parent $PSScriptRoot
$OriginalLocation = Get-Location
$TranscriptStarted = $false
$TemporaryFiles = @()
$StagingDirectories = @()
$Toolchain = 'nightly-2026-09-14'

function Run-Native([string]$Exe, [string[]]$Arguments) {
    & $Exe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Exe failed (exit $LASTEXITCODE)." }
}
function Run-Msvc([string]$Setup, [string]$HostTarget, [string]$Body, [string]$EncodedFlags = '') {
    # Use a child cmd process so compiler environment changes never leak
    # between architectures or into the user's shell.
    $Temporary = Join-Path ([IO.Path]::GetTempPath()) ('phiraiad-' + [guid]::NewGuid().ToString('N') + '.cmd')
    $script:TemporaryFiles += $Temporary
    $Text = "@echo off`r`nchcp 65001 >nul`r`ncall `"$Setup`" $HostTarget`r`nif errorlevel 1 exit /b 1`r`n" + $Body
    [IO.File]::WriteAllText($Temporary, $Text, (New-Object Text.UTF8Encoding($false)))
    $PreviousFlags = $env:CARGO_ENCODED_RUSTFLAGS
    try {
        if ($EncodedFlags) { $env:CARGO_ENCODED_RUSTFLAGS = $EncodedFlags }
        Run-Native $env:ComSpec @('/d', '/c', $Temporary)
    } finally { $env:CARGO_ENCODED_RUSTFLAGS = $PreviousFlags }
}
function Fill-MissingAssets([string]$Apk) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $Archive = [IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $Apk).Path)
    try {
        $AssetsRoot = [IO.Path]::GetFullPath((Join-Path $Root 'assets')) + [IO.Path]::DirectorySeparatorChar
        foreach ($Entry in $Archive.Entries) {
            if (-not $Entry.FullName.StartsWith('assets/', [StringComparison]::Ordinal) -or $Entry.FullName.EndsWith('/')) { continue }
            $Destination = [IO.Path]::GetFullPath((Join-Path $Root $Entry.FullName))
            if (-not $Destination.StartsWith($AssetsRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe APK asset path.' }
            if (Test-Path -LiteralPath $Destination) { continue }
            [IO.Directory]::CreateDirectory((Split-Path -Parent $Destination)) | Out-Null
            [IO.Compression.ZipFileExtensions]::ExtractToFile($Entry, $Destination, $false)
        }
    } finally { $Archive.Dispose() }
}

try {
    if ($env:OS -ne 'Windows_NT') { throw 'Run this script on Windows, not inside WSL.' }
    if ($Version -notmatch '^\d+\.\d+\.\d+(?:[-.][A-Za-z0-9.-]+)?$') { throw 'Invalid release version.' }
    Set-Location -LiteralPath $Root
    $CargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
    if (Test-Path -LiteralPath $CargoBin) { $env:PATH = "$CargoBin;$env:PATH" }
    foreach ($Name in @('git', 'rustup', 'cargo')) {
        $Command = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue
        if (-not $Command) { throw "Missing $Name. Install the Windows tools, then reopen the terminal." }
        Write-Host "[OK] $Name : $($Command.Source)"
    }
    $Vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $Vswhere)) { throw 'Visual Studio Build Tools not found. Install Desktop development with C++ and Windows SDK.' }
    $VisualStudio = & $Vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($LASTEXITCODE -ne 0 -or -not $VisualStudio) { throw 'MSVC x64/x86 component is missing.' }
    $Setup = Join-Path ([string]$VisualStudio) 'VC\Auxiliary\Build\vcvarsall.bat'
    if (-not (Test-Path -LiteralPath $Setup)) { throw 'vcvarsall.bat not found.' }
    $ArchList = if ($Architecture -eq 'all') { @('x64', 'arm64') } else { @($Architecture) }
    foreach ($Arch in $ArchList) {
        $HostTarget = if ($Arch -eq 'arm64') { 'x64_arm64' } else { 'x64' }
        Run-Msvc $Setup $HostTarget "where cl.exe`r`nif errorlevel 1 exit /b 1`r`nwhere link.exe`r`nif errorlevel 1 exit /b 1`r`nif not defined WindowsSdkDir exit /b 1`r`n"
        Write-Host "[OK] MSVC / SDK for $Arch"
    }
    $CargoText = Get-Content -LiteralPath (Join-Path $Root 'Cargo.toml') -Raw
    $SharedText = Get-Content -LiteralPath (Join-Path $Root 'xcode\Shared.xcconfig') -Raw
    $CargoVersion = [regex]::Match($CargoText, '(?ms)^\[workspace\.package\]\s*.*?^version\s*=\s*"([^"]+)"').Groups[1].Value
    $SharedVersion = [regex]::Match($SharedText, '(?m)^\s*MARKETING_VERSION\s*=\s*(\S+)').Groups[1].Value
    if (-not $CargoVersion -or $CargoVersion -ne $SharedVersion) { throw 'Cargo and xcode/Shared.xcconfig versions must match.' }
    $RequiredAssets = @('assets\font.ttf', 'assets\bold.ttf', 'assets\phigros.ttf', 'assets\background.jpg', 'assets\res\bgm')
    if ($CheckOnly) {
        foreach ($Asset in $RequiredAssets) {
            if (Test-Path -LiteralPath (Join-Path $Root $Asset)) { Write-Host "[OK] $Asset" }
            else { Write-Warning "Missing $Asset. Supply -BaseApk when building, or fill assets first." }
        }
        Write-Host 'Environment checks passed. No packages were downloaded or compiled.'
        exit 0
    }
    if ($BaseApk) { Fill-MissingAssets $BaseApk }
    foreach ($Asset in $RequiredAssets) {
        if (-not (Test-Path -LiteralPath (Join-Path $Root $Asset))) { throw "Missing $Asset. Supply -BaseApk with the original base APK or fill the assets directory." }
    }
    $Dist = Join-Path $Root 'dist'
    [IO.Directory]::CreateDirectory($Dist) | Out-Null
    $Log = Join-Path $Dist ('windows-build-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.log')
    Start-Transcript -LiteralPath $Log | Out-Null
    $TranscriptStarted = $true
    if (-not $VcpkgRoot) { $VcpkgRoot = Join-Path $env:LOCALAPPDATA 'PhiraiAd-build\vcpkg' }
    $VcpkgRoot = [IO.Path]::GetFullPath($VcpkgRoot)
    if (-not (Test-Path -LiteralPath $VcpkgRoot)) {
        Run-Native 'git' @('clone', 'https://github.com/microsoft/vcpkg.git', $VcpkgRoot)
    }
    $Vcpkg = Join-Path $VcpkgRoot 'vcpkg.exe'
    if (-not (Test-Path -LiteralPath $Vcpkg)) {
        $Bootstrap = Join-Path $VcpkgRoot 'bootstrap-vcpkg.bat'
        if (-not (Test-Path -LiteralPath $Bootstrap)) { throw 'VcpkgRoot is not a vcpkg checkout.' }
        Run-Native $env:ComSpec @('/d', '/c', $Bootstrap, '-disableMetrics')
    }
    Run-Native 'rustup' @('toolchain', 'install', $Toolchain, '--profile', 'minimal')
    $Built = @()
    foreach ($Arch in $ArchList) {
        $Target = if ($Arch -eq 'arm64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
        $Triplet = if ($Arch -eq 'arm64') { 'arm64-windows-static' } else { 'x64-windows-static' }
        $HostTarget = if ($Arch -eq 'arm64') { 'x64_arm64' } else { 'x64' }
        $Lib = Join-Path $VcpkgRoot "installed\$Triplet\lib"
        $EncodedFlags = @('-C', 'target-feature=+crt-static', '-L', "native=$Lib") -join [char]31
        # Dollar expansion is PowerShell only; batch values are fully quoted.
        $Body = @"
setlocal
set "VCPKG_DISABLE_METRICS=1"
set "RUSTFLAGS="
set "CARGO_TARGET_DIR=$Root\target\windows"
"$Vcpkg" install zlib:$Triplet
if errorlevel 1 exit /b 1
for %%F in (zs.lib zlibstatic.lib zlib.lib) do if exist "$Lib\%%F" copy /Y "$Lib\%%F" "$Lib\z.lib" >nul
if not exist "$Lib\z.lib" exit /b 1
rustup target add --toolchain $Toolchain $Target
if errorlevel 1 exit /b 1
cargo +$Toolchain build --locked -p phira-main --bin phira-main --release --target $Target
if errorlevel 1 exit /b 1
exit /b 0
"@
        Write-Host "Building $Target..."
        Run-Msvc $Setup $HostTarget $Body $EncodedFlags
        $Exe = Join-Path $Root "target\windows\$Target\release\phira-main.exe"
        if (-not (Test-Path -LiteralPath $Exe)) { throw "Build output missing: $Exe" }
        $Id = [guid]::NewGuid().ToString('N')
        $Stage = Join-Path $Dist ('.windows-stage-' + $Id)
        $StagingDirectories += $Stage
        $Folder = "PhiraiAd-v$Version-windows-$Arch"
        $Package = Join-Path $Stage $Folder
        [IO.Directory]::CreateDirectory($Package) | Out-Null
        Copy-Item -LiteralPath $Exe -Destination (Join-Path $Package 'PhiraiAd.exe')
        Copy-Item -LiteralPath (Join-Path $Root 'assets') -Destination $Package -Recurse
        $Revision = & git rev-parse HEAD
        $Manifest = [ordered]@{ version = $Version; target = $Target; revision = [string]$Revision; executableSha256 = (Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash; deviceTested = $false }
        $Manifest | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Package 'build-info.json') -Encoding UTF8
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $ArchiveTemporary = Join-Path $Dist ('.archive-' + $Id + '.zip')
        $TemporaryFiles += $ArchiveTemporary
        [IO.Compression.ZipFile]::CreateFromDirectory($Stage, $ArchiveTemporary)
        $Archive = Join-Path $Dist ($Folder + '.zip')
        Move-Item -LiteralPath $ArchiveTemporary -Destination $Archive -Force
        $Built += $Archive
    }
    Write-Host 'Build and packaging completed:'
    $Built | ForEach-Object { Write-Host $_ }
    Write-Host "Log: $Log"
} catch {
    Write-Host ('FAILED: ' + $_.Exception.Message) -ForegroundColor Red
    exit 1
} finally {
    if ($TranscriptStarted) { Stop-Transcript | Out-Null }
    foreach ($File in $TemporaryFiles) { if (Test-Path -LiteralPath $File) { Remove-Item -LiteralPath $File -Force } }
    foreach ($Directory in $StagingDirectories) { if (Test-Path -LiteralPath $Directory) { Remove-Item -LiteralPath $Directory -Recurse -Force } }
    Set-Location $OriginalLocation
}
