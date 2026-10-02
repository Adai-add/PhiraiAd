# Windows PowerShell 5.1+. Run on Windows; WSL is the source of truth.
[CmdletBinding()]
param([switch]$CheckOnly, [string]$ConfigPath = '')
Write-Host '[START] PowerShell script loaded.'
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$OriginalLocation = Get-Location
$Distribution = 'Ubuntu'
$WslRoot = '/home/adai_add/phira-build'
$WindowsRoot = 'E:\PhiraiAd-windows'
$OutputRoot = 'C:\Users\admin\Desktop\phira-rep'
$SourceRoot = "\\wsl.localhost\$Distribution\home\adai_add\phira-build"
$RunId = Get-Date -Format 'yyyyMMdd-HHmmss'
$Results = @()
$TemporaryConfig = ''
$LockStream = $null
$PreviousHttpProxy = $env:HTTP_PROXY
$PreviousHttpsProxy = $env:HTTPS_PROXY

function Invoke-Checked([string]$Exe, [string[]]$Arguments) {
    # Rust sends ordinary progress to stderr. PS 5.1 must not treat it as fatal.
    $PreviousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        & $Exe @Arguments
        $Code = $LASTEXITCODE
    } finally { $ErrorActionPreference = $PreviousPreference }
    if ($Code -ne 0) { throw "$Exe failed (exit $Code)." }
}
function Probe-Wsl([int]$TimeoutSeconds = 30) {
    $Info = New-Object Diagnostics.ProcessStartInfo
    $Info.FileName = 'wsl.exe'
    $Info.Arguments = '-d Ubuntu -- true'
    $Info.UseShellExecute = $false
    $Info.WorkingDirectory = $env:SystemRoot
    $Process = New-Object Diagnostics.Process
    $Process.StartInfo = $Info
    try {
        if (-not $Process.Start()) { throw 'Unable to start wsl.exe.' }
        if (-not $Process.WaitForExit($TimeoutSeconds * 1000)) {
            try { $Process.Kill() } catch {}
            throw "WSL startup timed out after $TimeoutSeconds seconds. Test wsl -d Ubuntu -- echo WSL_OK in another terminal."
        }
        if ($Process.ExitCode -ne 0) { throw "WSL probe failed (exit $($Process.ExitCode))." }
    } finally { $Process.Dispose() }
}
function Check-NotLink([string]$Path) {
    if (Test-Path -LiteralPath $Path) {
        $Item = Get-Item -LiteralPath $Path -Force
        if (($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Refusing to synchronize/delete a link: $Path"
        }
    }
}
function Invoke-Worker([string]$Action) {
    # JSON has paths/settings only. Password never appears in an argument.
    Invoke-Checked 'wsl.exe' @('-d', $Distribution, '--', 'bash', '-lc',
        'export PATH="$HOME/.cargo/bin:$PATH"; exec python3 "$1/scripts/build_all_new_wsl.py" "$2" "$3"',
        'phiraiad-build', $WslRoot, $Action, $TemporaryConfig)
}
function Sync-Source([switch]$Preview) {
    Check-NotLink $WindowsRoot
    $ExcludeDirectories = @('.git', '.cargo', 'target', 'dist', 'cache', 'data',
        '.idea', '.vscode', '__pycache__', '签名', 'static-lib')
    $ExcludeFiles = @('.env', '.env.*', '*.jks', '*.keystore', '*.p12', '*.pfx',
        '*.pem', '*.key', 'key.txt', 'config.txt', 'monitor-config.yml',
        'local.properties', 'keystore.properties', 'signing.properties',
        'LocalSigning.xcconfig', 'build_all_new.config.json',
        'build_windows_release.ps1', 'build_windows.cmd', '*.log', '.build-all-settings-*.json')
    $Arguments = @($SourceRoot, $WindowsRoot, '/MIR', '/XJ', '/R:1', '/W:1', '/NP',
        '/XD') + $ExcludeDirectories + @('/XF') + $ExcludeFiles
    if ($Preview) { $Arguments += '/L' }
    & robocopy.exe @Arguments
    if ($LASTEXITCODE -ge 8) { throw "Source sync failed (robocopy exit $LASTEXITCODE)." }
}
function Clear-WindowsTarget {
    $Target = Join-Path $WindowsRoot 'target'
    Check-NotLink $Target
    if (Test-Path -LiteralPath $Target) {
        Remove-Item -LiteralPath $Target -Recurse -Force
    }
}
function Build-Platform([string]$Name, [scriptblock]$Task) {
    $LogPath = Join-Path $LogRoot ($Name + '.log')
    try {
        Write-Host "========== $Name =========="
        & $Task 2>&1 | Tee-Object -FilePath $LogPath | Out-Host
        $script:Results += [pscustomobject]@{ Platform = $Name; Status = 'OK'; Log = $LogPath }
    } catch {
        $Message = $_.Exception.Message
        Write-Host "$Name FAILED: $Message" -ForegroundColor Red
        Add-Content -LiteralPath $LogPath -Value "FAILED: $Message"
        $script:Results += [pscustomobject]@{ Platform = $Name; Status = 'FAILED'; Log = $LogPath }
    }
}

try {
    if ($env:OS -ne 'Windows_NT') { throw 'Run build_all_new.cmd on Windows.' }
    # Start WSL before accessing its UNC share.
    Write-Host '[1/7] Checking WSL (30 second timeout)...'
    Probe-Wsl
    Write-Host '[2/7] Checking source files on the WSL share...'
    foreach ($Relative in @('Cargo.toml', 'scripts\build_all_new_wsl.py', 'scripts\package_android_release.py')) {
        if (-not (Test-Path -LiteralPath (Join-Path $SourceRoot $Relative))) {
            throw "Missing $SourceRoot\$Relative. Extract the bundle into the WSL source directory first."
        }
    }
    Write-Host '[3/7] Loading build configuration...'
    if (-not $ConfigPath) { $ConfigPath = Join-Path $SourceRoot 'build_all_new.config.json' }
    if (-not (Test-Path -LiteralPath $ConfigPath)) {
        $Defaults = [ordered]@{
            baseApk = '/mnt/c/Users/admin/Desktop/phira-rep/签名/Phira-Replica-0.8.2-r23.0.apk'
            keystore = '/mnt/c/Users/admin/Desktop/phira-rep/签名/phira-replica-release.jks'
            keyAlias = 'phira-replica'
            passwordFile = '/mnt/c/Users/admin/Desktop/phira-rep/签名/key.txt'
            ndk = '/home/adai_add/Android/Sdk/ndk/27.3.13750724'
            buildTools = '/home/adai_add/Android/Sdk/build-tools/35.0.0'
            vcpkgRoot = 'E:\PhiraiAd-vcpkg'
            version = '1.1.0'
            minimumVersionCode = 11000
            proxy = 'http://127.0.0.1:51081'
        }
        [IO.File]::WriteAllText($ConfigPath, ($Defaults | ConvertTo-Json), (New-Object Text.UTF8Encoding($false)))
    }
    $Config = Get-Content -LiteralPath $ConfigPath -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($Name in @('baseApk', 'keystore', 'keyAlias', 'passwordFile', 'ndk', 'buildTools',
        'vcpkgRoot', 'version', 'minimumVersionCode', 'proxy')) {
        if (-not ($Config.PSObject.Properties.Name -contains $Name)) { throw "Config missing $Name." }
    }
    if ($Config.version -ne '1.1.0') { throw 'This release script expects version 1.1.0.' }
    New-Item -ItemType Directory -Path $OutputRoot -Force | Out-Null
    $LogRoot = Join-Path $OutputRoot ('build-logs\' + $RunId)
    New-Item -ItemType Directory -Path $LogRoot -Force | Out-Null
    # Reject concurrent runs; FileShare.None releases automatically on process exit.
    $LockStream = [IO.File]::Open((Join-Path $OutputRoot 'build-all-new.lock'),
        [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    $Config | Add-Member -NotePropertyName root -NotePropertyValue $WslRoot -Force
    $Config | Add-Member -NotePropertyName output -NotePropertyValue '/mnt/c/Users/admin/Desktop/phira-rep' -Force
    $TempName = '.build-all-settings-' + [guid]::NewGuid().ToString('N') + '.json'
    $TempWindows = Join-Path $SourceRoot $TempName
    $TemporaryConfig = "$WslRoot/$TempName"
    [IO.File]::WriteAllText($TempWindows, ($Config | ConvertTo-Json), (New-Object Text.UTF8Encoding($false)))

    if ($CheckOnly) {
        Invoke-Worker 'check'
        Sync-Source -Preview
        Write-Host 'Check completed. No synchronization, cleanup, or compilation performed.'
        exit 0
    }
    # Windows tool scripts are also supplied in this bundle. Keep destination copies
    # if the WSL source no longer has them, but refresh when present.
    Write-Host '[4/7] Synchronizing WSL source to E:\PhiraiAd-windows...'
    Sync-Source 2>&1 | Tee-Object -FilePath (Join-Path $LogRoot 'sync.log') | Out-Host
    foreach ($Relative in @('scripts\build_windows_release.ps1', 'build_windows.cmd')) {
        $SourceFile = Join-Path $SourceRoot $Relative
        if (Test-Path -LiteralPath $SourceFile) {
            $DestinationFile = Join-Path $WindowsRoot $Relative
            New-Item -ItemType Directory -Path (Split-Path $DestinationFile -Parent) -Force | Out-Null
            Copy-Item -LiteralPath $SourceFile -Destination $DestinationFile -Force
        }
    }
    Write-Host '[5/7] Removing Windows target build cache (may take a while)...'
    Clear-WindowsTarget
    Write-Host '[6/7] Removing WSL target build cache (may take a while)...'
    Invoke-Worker 'clean'
    Write-Host '[7/7] Starting platform builds...'
    # Dependency downloads are preserved. Use the existing Windows local proxy only
    # when reachable; WSL inherits its own working proxy settings separately.
    if ($Config.proxy) {
        $ProxyUri = [Uri]$Config.proxy
        $Client = New-Object Net.Sockets.TcpClient
        try {
            $Connection = $Client.ConnectAsync($ProxyUri.Host, $ProxyUri.Port)
            if ($Connection.Wait(1500) -and $Client.Connected) {
                $env:HTTP_PROXY = $Config.proxy; $env:HTTPS_PROXY = $Config.proxy
                Write-Host "Using Windows proxy: $($Config.proxy)"
            }
        } catch { Write-Host 'Configured Windows proxy unavailable; preserving existing proxy settings.' }
        finally { $Client.Dispose() }
    }
    Build-Platform 'android-arm64' { Invoke-Worker 'android' }
    Build-Platform 'windows-x64' {
        $Builder = Join-Path $WindowsRoot 'scripts\build_windows_release.ps1'
        Invoke-Checked 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
            $Builder, '-Architecture', 'x64', '-VcpkgRoot', [string]$Config.vcpkgRoot, '-Version', '1.1.0')
        $Built = Join-Path $WindowsRoot 'dist\PhiraiAd-v1.1.0-windows-x64.zip'
        if (-not (Test-Path -LiteralPath $Built)) { throw 'Windows ZIP missing.' }
        $TemporaryOutput = Join-Path $OutputRoot ('.windows-' + $RunId + '.zip')
        try {
            Copy-Item -LiteralPath $Built -Destination $TemporaryOutput
            Move-Item -LiteralPath $TemporaryOutput -Destination (Join-Path $OutputRoot 'PhiraiAd-windows_x64-new.zip') -Force
        } finally {
            if (Test-Path -LiteralPath $TemporaryOutput) { Remove-Item -LiteralPath $TemporaryOutput -Force }
        }
    }
    Build-Platform 'linux-x64' { Invoke-Worker 'linux' }
    $Results | Format-Table -AutoSize | Out-Host
    $Results | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $LogRoot 'results.json') -Encoding UTF8
    Write-Host "Outputs: $OutputRoot"
    Write-Host "Logs: $LogRoot"
    if (@($Results | Where-Object Status -eq 'FAILED').Count -gt 0) { exit 1 }
} catch {
    Write-Host ('FAILED: ' + $_.Exception.Message) -ForegroundColor Red
    exit 1
} finally {
    if ($TemporaryConfig) {
        $TemporaryPath = Join-Path $SourceRoot (Split-Path $TemporaryConfig -Leaf)
        if (Test-Path -LiteralPath $TemporaryPath) { Remove-Item -LiteralPath $TemporaryPath -Force }
    }
    if ($LockStream) { $LockStream.Dispose() }
    $env:HTTP_PROXY = $PreviousHttpProxy
    $env:HTTPS_PROXY = $PreviousHttpsProxy
    Set-Location $OriginalLocation
}
