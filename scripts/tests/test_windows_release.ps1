# Runs the actual packaging script with native compiler commands stubbed out.
# Requires PowerShell 7 on Linux/macOS.
param([string]$BuildScriptsDirectory = '')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$Repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $BuildScriptsDirectory) { $BuildScriptsDirectory = Join-Path $Repo 'scripts' }
$Pwsh = Join-Path $PSHOME 'pwsh'
$Temp = Join-Path ([IO.Path]::GetTempPath()) ('phiraiad-package-test-' + [guid]::NewGuid().ToString('N'))
$Saved = @{}
foreach ($Name in @('OS', 'USERPROFILE', 'ProgramFiles(x86)', 'ComSpec', 'PATH')) {
    $Saved[$Name] = [Environment]::GetEnvironmentVariable($Name)
}
function Assert($Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Write-File([string]$Path, [string]$Text) {
    [IO.Directory]::CreateDirectory((Split-Path -Parent $Path)) | Out-Null
    [IO.File]::WriteAllText($Path, $Text)
}
try {
    # The test deliberately avoids invoking MSVC, vcpkg, or rustup. ZIP creation,
    # config loading, release naming, manifests, and the multi-builder are real.
    if ($IsWindows) { throw 'This native-command fixture currently targets PowerShell on Unix.' }
    $Root = Join-Path $Temp 'source'
    [IO.Directory]::CreateDirectory($Root) | Out-Null
    & git -C $Root init --quiet
    & git -C $Root -c user.name=Fixture -c user.email=fixture@example.invalid -c commit.gpgsign=false commit --allow-empty -m fixture --quiet
    $Bin = Join-Path $Temp 'bin'
    foreach ($Name in @('cargo', 'rustup')) {
        $Path = Join-Path $Bin $Name
        Write-File $Path "#!/bin/sh`nexit 0`n"
        & chmod +x $Path
    }
    $Vs = Join-Path $Temp 'vs'
    $Vswhere = Join-Path $Temp 'programs/Microsoft Visual Studio/Installer/vswhere.exe'
    Write-File $Vswhere "#!/bin/sh`nprintf '%s\n' '$Vs'`n"
    & chmod +x $Vswhere
    Write-File (Join-Path $Vs 'VC/Auxiliary/Build/vcvarsall.bat') ''
    $Vcpkg = Join-Path $Temp 'vcpkg'
    Write-File (Join-Path $Vcpkg 'vcpkg.exe') "#!/bin/sh`nexit 0`n"
    & chmod +x (Join-Path $Vcpkg 'vcpkg.exe')
    $env:OS = 'Windows_NT'
    $env:USERPROFILE = $Temp
    [Environment]::SetEnvironmentVariable('ProgramFiles(x86)', (Join-Path $Temp 'programs'))
    $env:ComSpec = '/bin/true'
    $env:PATH = $Bin + [IO.Path]::PathSeparator + $env:PATH
    $Builder = Join-Path $Root 'scripts/build_windows_release.ps1'
    [IO.Directory]::CreateDirectory((Split-Path -Parent $Builder)) | Out-Null
    Copy-Item (Join-Path $BuildScriptsDirectory 'build_windows_release.ps1') $Builder
    Write-File (Join-Path $Root 'Cargo.toml') "[workspace.package]`nversion = `"0.8.2`"`n"
    Write-File (Join-Path $Root 'xcode/Shared.xcconfig') 'MARKETING_VERSION=0.8.2'
    foreach ($Asset in @('font.ttf', 'bold.ttf', 'phigros.ttf', 'background.jpg', 'res/bgm')) {
        Write-File (Join-Path $Root "assets/$Asset") 'asset fixture'
    }
    foreach ($Target in @('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')) {
        Write-File (Join-Path $Root "target/windows/$Target/release/phira-main.exe") 'compiler output fixture'
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $ConfigVersion = (Get-Content (Join-Path $Repo 'build_all_new.config.json') -Raw | ConvertFrom-Json).version
    foreach ($Case in @(
        @{ Config = $ConfigVersion; Override = ''; Expected = $ConfigVersion },
        @{ Config = '9.8.7'; Override = ''; Expected = '9.8.7' },
        @{ Config = '9.8.7'; Override = '2.0.0-rc.1'; Expected = '2.0.0-rc.1' }
    )) {
        Write-File (Join-Path $Root 'build_all_new.config.json') (@{ version = $Case.Config } | ConvertTo-Json)
        $Arguments = @('-NoProfile', '-File', $Builder, '-Architecture', 'all', '-VcpkgRoot', $Vcpkg)
        if ($Case.Override) { $Arguments += @('-Version', $Case.Override) }
        & $Pwsh @Arguments | Out-Host
        Assert ($LASTEXITCODE -eq 0) 'Packaging process failed'
        foreach ($Arch in @('x64', 'arm64')) {
            $Folder = "PhiraiAd-v$($Case.Expected)-windows-$Arch"
            $Archive = [IO.Compression.ZipFile]::OpenRead((Join-Path $Root "dist/$Folder.zip"))
            try {
                $Entry = $Archive.GetEntry("$Folder/build-info.json")
                Assert ($null -ne $Entry) 'ZIP root has the wrong release version'
                $Reader = [IO.StreamReader]::new($Entry.Open())
                try { $Manifest = $Reader.ReadToEnd() | ConvertFrom-Json } finally { $Reader.Dispose() }
                Assert ($Manifest.version -eq $Case.Expected) 'Manifest version mismatch'
                Assert ($null -ne $Archive.GetEntry("$Folder/PhiraiAd.exe")) 'Executable missing'
                Assert ($null -ne $Archive.GetEntry("$Folder/assets/font.ttf")) 'Assets missing'
            } finally { $Archive.Dispose() }
        }
    }
    # Execute the original windows-x64 scriptblock from the multi-platform builder.
    $Tokens = $null; $Errors = $null
    $Ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $BuildScriptsDirectory 'build_all_new.ps1'), [ref]$Tokens, [ref]$Errors)
    Assert ($Errors.Count -eq 0) 'Multi-platform builder syntax error'
    $Command = $Ast.Find({ param($Node)
        $Node -is [Management.Automation.Language.CommandAst] -and $Node.GetCommandName() -eq 'Build-Platform' -and
        $Node.CommandElements[1].Extent.Text -eq "'windows-x64'"
    }, $true)
    Assert ($null -ne $Command) 'Windows build task not found'
    $WindowsRoot = $Root; $OutputRoot = Join-Path $Temp 'output'; $RunId = 'fixture'
    [IO.Directory]::CreateDirectory($OutputRoot) | Out-Null
    $Config = [pscustomobject]@{ version = '9.8.7'; vcpkgRoot = $Vcpkg }
    function Invoke-Checked([string]$Exe, [string[]]$Arguments) {
        $Index = [Array]::IndexOf($Arguments, '-Version')
        Assert ($Index -ge 0 -and $Arguments[$Index + 1] -eq '9.8.7') 'Multi-builder passed a stale version'
    }
    & ($Command.CommandElements[2].ScriptBlock.GetScriptBlock())
    Assert (Test-Path (Join-Path $OutputRoot 'PhiraiAd-windows_x64-new.zip')) 'Multi-builder selected the wrong archive'
    Write-Host 'PASS: 6 ZIP roots/manifests (config/default/override, x64/arm64) and multi-builder version propagation'
} finally {
    foreach ($Name in $Saved.Keys) { [Environment]::SetEnvironmentVariable($Name, $Saved[$Name]) }
    if (Test-Path $Temp) { Remove-Item -LiteralPath $Temp -Recurse -Force }
}
