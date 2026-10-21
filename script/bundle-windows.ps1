[CmdletBinding()]
Param(
    [Parameter()][Alias('i')][switch]$Install,
    [Parameter()][Alias('h')][switch]$Help,
    [Parameter()][Alias('a')][string]$Architecture
)

if ($Help) {
    Write-Output "Usage: bundle-windows.ps1 [-Architecture <x86_64|aarch64>] [-Install] [-Help]"
    Write-Output "Build the Fanta installer for Windows into target\Fanta-<architecture>.exe."
    Write-Output ""
    Write-Output "Options:"
    Write-Output "  -Architecture, -a Which architecture to build (x86_64 or aarch64)"
    Write-Output "  -Install, -i      Run the installer after building."
    Write-Output "  -Help, -h         Show this help message."
    Write-Output ""
    Write-Output "The installer is unsigned unless FANTA_WINDOWS_SIGN=1, which signs the binaries and the"
    Write-Output "installer with Azure Trusted Signing (crates\fanta\resources\windows\sign.ps1)."
    exit 0
}

. "$PSScriptRoot/lib/workspace.ps1"

# https://stackoverflow.com/questions/57949031/powershell-script-stops-if-program-fails-like-bash-set-o-errexit
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
# Invoke-WebRequest downloads far more slowly while it draws a progress bar.
$ProgressPreference = 'SilentlyContinue'

$buildSuccess = $false
$installerPath = $null
$sign = $env:FANTA_WINDOWS_SIGN -eq "1"

$OSArchitecture = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
    "X64" { "x86_64" }
    "Arm64" { "aarch64" }
    default { throw "Unsupported architecture" }
}

$Architecture = if ($Architecture) {
    $Architecture
} else {
    $OSArchitecture
}

$target = "$Architecture-pc-windows-msvc"

function Get-VSArch {
    param(
        [string]$Arch
    )

    switch ($Arch) {
        "x86_64" { "amd64" }
        "aarch64" { "arm64" }
    }
}

function EnterVsDevShell {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vswhere)) {
        throw "vswhere.exe is missing. Install Visual Studio 2022 or its Build Tools with the Desktop development with C++ workload."
    }
    $component = switch ($Architecture) {
        "x86_64" { "Microsoft.VisualStudio.Component.VC.Tools.x86.x64" }
        "aarch64" { "Microsoft.VisualStudio.Component.VC.Tools.ARM64" }
    }
    $vsPath = & $vswhere -latest -products * -requires $component -property installationPath
    if (-not $vsPath) {
        throw "No Visual Studio installation has the C++ build tools for $Architecture ($component)."
    }
    Write-Output "Using Visual Studio at $vsPath"

    # Launch-VsDevShell.ps1 changes the current directory.
    Push-Location
    & "$vsPath\Common7\Tools\Launch-VsDevShell.ps1" -Arch (Get-VSArch -Arch $Architecture) -HostArch (Get-VSArch -Arch $OSArchitecture)
    Pop-Location
}

function CheckEnvironmentVariables {
    if (-not $sign) {
        return
    }

    $requiredVars = @(
        'AZURE_TENANT_ID', 'AZURE_CLIENT_ID', 'AZURE_CLIENT_SECRET',
        'ACCOUNT_NAME', 'CERT_PROFILE_NAME', 'ENDPOINT',
        'FILE_DIGEST', 'TIMESTAMP_DIGEST', 'TIMESTAMP_SERVER'
    )

    foreach ($var in $requiredVars) {
        if (-not (Test-Path "env:$var")) {
            Write-Error "$var is not set, and FANTA_WINDOWS_SIGN=1 needs it"
            exit 1
        }
    }
}

function PrepareForBundle {
    if (Test-Path "$innoDir") {
        Remove-Item -Path "$innoDir" -Recurse -Force
    }
    New-Item -Path "$innoDir" -ItemType Directory -Force
    Copy-Item -Path "$env:FANTA_WORKSPACE\crates\fanta\resources\windows\*" -Destination "$innoDir" -Recurse -Force
    New-Item -Path "$innoDir\bin" -ItemType Directory -Force
    New-Item -Path "$innoDir\tools" -ItemType Directory -Force
    New-Item -Path "$downloadDir" -ItemType Directory -Force

    rustup target add $target
}

function GenerateLicenses {
    . $PSScriptRoot/generate-licenses.ps1
}

function BuildFantaAndItsFriends {
    Write-Output "Building Fanta and its friends, for channel: $channel"
    # Build fanta.exe, cli.exe and auto_update_helper.exe
    cargo build --release --locked --package fanta --package cli --package auto_update_helper --target $target
    Copy-Item -Path "$CargoOutDir\fanta.exe" -Destination "$innoDir\Fanta.exe" -Force
    Copy-Item -Path "$CargoOutDir\cli.exe" -Destination "$innoDir\cli.exe" -Force
    Copy-Item -Path "$CargoOutDir\auto_update_helper.exe" -Destination "$innoDir\auto_update_helper.exe" -Force
}

function ZipFantaAndItsFriendsDebug {
    $items = @(
        "$CargoOutDir\fanta.pdb",
        "$CargoOutDir\cli.pdb",
        "$CargoOutDir\auto_update_helper.pdb"
    )

    Compress-Archive -Path $items -DestinationPath $debugArchive -Force
}


function UploadToSentry {
    if (-not (Get-Command "sentry-cli" -ErrorAction SilentlyContinue)) {
        Write-Output "sentry-cli not found. skipping sentry upload."
        Write-Output "install with: 'winget install -e --id=Sentry.sentry-cli'"
        return
    }
    if (-not (Test-Path "env:SENTRY_AUTH_TOKEN")) {
        Write-Output "missing SENTRY_AUTH_TOKEN. skipping sentry upload."
        return
    }
    Write-Output "Uploading fanta debug symbols to sentry..."
    for ($i = 1; $i -le 3; $i++) {
        try {
            sentry-cli debug-files upload --include-sources --wait -p fanta -o fanta-dev $CargoOutDir
            break
        }
        catch {
            Write-Output "Sentry upload attempt $i failed: $_"
            if ($i -eq 3) {
                Write-Output "All sentry upload attempts failed"
                throw
            }
            Start-Sleep -Seconds 2
        }
    }
}

function SignFantaAndItsFriends {
    if (-not $sign) {
        return
    }

    $files = "$innoDir\Fanta.exe,$innoDir\cli.exe,$innoDir\auto_update_helper.exe"
    & "$innoDir\sign.ps1" $files
}

function DownloadAMDGpuServices {
    # GPUI loads AGS on AMD GPUs only to read the driver version and works without it, so a
    # failed download leaves the DLL out of the installer instead of failing the build.
    # If you update the AGS SDK version, please also update the GPUI renderer in squidred-dev/fanta-ui
    $url = "https://codeload.github.com/GPUOpen-LibrariesAndSDKs/AGS_SDK/zip/refs/tags/v6.3.0"
    $zipPath = "$downloadDir\AGS_SDK_v6.3.0.zip"
    try {
        Invoke-WebRequest -Uri $url -OutFile $zipPath
        Expand-Archive -Path $zipPath -DestinationPath "$downloadDir" -Force
        Copy-Item -Path "$downloadDir\AGS_SDK-6.3.0\ags_lib\lib\amd_ags_x64.dll" -Destination "$innoDir\amd_ags_x64.dll" -Force
    }
    catch {
        Write-Warning "Building the installer without amd_ags_x64.dll: $_"
    }
}

function DownloadConpty {
    # Keep this the ConPTY release that crates/fanta/build.rs puts next to development builds.
    $url = "https://github.com/microsoft/terminal/releases/download/v1.24.10621.0/Microsoft.Windows.Console.ConPTY.1.24.260303001.nupkg"
    # Expand-Archive accepts only a .zip extension.
    $zipPath = "$downloadDir\conpty.nupkg.zip"
    Invoke-WebRequest -Uri $url -OutFile $zipPath
    Expand-Archive -Path $zipPath -DestinationPath "$downloadDir\conpty" -Force
}

function CollectFiles {
    Move-Item -Path "$innoDir\cli.exe" -Destination "$innoDir\bin\fanta.exe" -Force
    Move-Item -Path "$innoDir\fanta.sh" -Destination "$innoDir\bin\fanta" -Force
    Move-Item -Path "$innoDir\auto_update_helper.exe" -Destination "$innoDir\tools\auto_update_helper.exe" -Force
    if($Architecture -eq "aarch64") {
        New-Item -Type Directory -Path "$innoDir\arm64" -Force
        Move-Item -Path "$downloadDir\conpty\build\native\runtimes\arm64\OpenConsole.exe" -Destination "$innoDir\arm64\OpenConsole.exe" -Force
        Move-Item -Path "$downloadDir\conpty\runtimes\win-arm64\native\conpty.dll" -Destination "$innoDir\conpty.dll" -Force
    }
    else {
        New-Item -Type Directory -Path "$innoDir\x64" -Force
        New-Item -Type Directory -Path "$innoDir\arm64" -Force
        Move-Item -Path "$downloadDir\conpty\build\native\runtimes\x64\OpenConsole.exe" -Destination "$innoDir\x64\OpenConsole.exe" -Force
        Move-Item -Path "$downloadDir\conpty\build\native\runtimes\arm64\OpenConsole.exe" -Destination "$innoDir\arm64\OpenConsole.exe" -Force
        Move-Item -Path "$downloadDir\conpty\runtimes\win-x64\native\conpty.dll" -Destination "$innoDir\conpty.dll" -Force
    }
}

function FindInnoSetupCompiler {
    $command = Get-Command "ISCC.exe" -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }
    # The windows-2025 runner image has no Inno Setup on PATH: https://github.com/actions/runner-images/issues/11228
    foreach ($root in @(${env:ProgramFiles(x86)}, $env:ProgramFiles, "$env:LOCALAPPDATA\Programs")) {
        $candidate = "$root\Inno Setup 6\ISCC.exe"
        if ($root -and (Test-Path $candidate)) {
            return $candidate
        }
    }
    throw "Inno Setup 6 is not installed. Install it with: winget install -e --id JRSoftware.InnoSetup"
}

function BuildInstaller {
    $issFilePath = "$innoDir\fanta.iss"
    switch ($channel) {
        "stable" {
            $appId = "{{2DB0DA96-CA55-49BB-AF4F-64AF36A86712}"
            $appIconName = "app-icon"
            $appName = "Fanta"
            $appDisplayName = "Fanta"
            $appSetupName = "Fanta-$Architecture"
            # The mutex name here should match the mutex name in crates\fanta\src\fanta\windows_only_instance.rs
            $appMutex = "Fanta-Stable-Instance-Mutex"
            $appExeName = "Fanta"
            $regValueName = "Fanta"
            $appUserId = "Fanta.Fanta"
        }
        "preview" {
            $appId = "{{F70E4811-D0E2-4D88-AC99-D63752799F95}"
            $appIconName = "app-icon-preview"
            $appName = "Fanta Preview"
            $appDisplayName = "Fanta Preview"
            $appSetupName = "Fanta-$Architecture"
            # The mutex name here should match the mutex name in crates\fanta\src\fanta\windows_only_instance.rs
            $appMutex = "Fanta-Preview-Instance-Mutex"
            $appExeName = "Fanta"
            $regValueName = "FantaPreview"
            $appUserId = "Fanta.Fanta.Preview"
        }
        "nightly" {
            $appId = "{{1BDB21D3-14E7-433C-843C-9C97382B2FE0}"
            $appIconName = "app-icon-nightly"
            $appName = "Fanta Nightly"
            $appDisplayName = "Fanta Nightly"
            $appSetupName = "Fanta-$Architecture"
            # The mutex name here should match the mutex name in crates\fanta\src\fanta\windows_only_instance.rs
            $appMutex = "Fanta-Nightly-Instance-Mutex"
            $appExeName = "Fanta"
            $regValueName = "FantaNightly"
            $appUserId = "Fanta.Fanta.Nightly"
        }
        "dev" {
            $appId = "{{8357632E-24A4-4F32-BA97-E575B4D1FE5D}"
            $appIconName = "app-icon-dev"
            $appName = "Fanta Dev"
            $appDisplayName = "Fanta Dev"
            $appSetupName = "Fanta-$Architecture"
            # The mutex name here should match the mutex name in crates\fanta\src\fanta\windows_only_instance.rs
            $appMutex = "Fanta-Dev-Instance-Mutex"
            $appExeName = "Fanta"
            $regValueName = "FantaDev"
            $appUserId = "Fanta.Fanta.Dev"
        }
        default {
            Write-Error "can't bundle installer for $channel."
            exit 1
        }
    }

    $innoSetupPath = FindInnoSetupCompiler

    $definitions = @{
        "AppId"              = $appId
        "AppIconName"        = $appIconName
        "OutputDir"          = "$env:FANTA_WORKSPACE\target"
        "AppSetupName"       = $appSetupName
        "AppName"            = $appName
        "AppDisplayName"     = $appDisplayName
        "RegValueName"       = $regValueName
        "AppMutex"           = $appMutex
        "AppExeName"         = $appExeName
        "ResourcesDir"       = "$innoDir"
        "AppUserId"          = $appUserId
        "Version"            = "$env:RELEASE_VERSION"
        # Windows version resources take numbers only: 2.1.0-rc.1 becomes 2.1.0.
        "VersionInfoVersion" = ($env:RELEASE_VERSION -split '[-+]')[0]
        "SourceDir"          = "$env:FANTA_WORKSPACE"
    }

    $defs = @()
    foreach ($key in $definitions.Keys) {
        $defs += "/d$key=`"$($definitions[$key])`""
    }

    $innoArgs = @($issFilePath) + $defs
    if ($sign) {
        $signTool = "powershell.exe -ExecutionPolicy Bypass -File $innoDir\sign.ps1 `$f"
        $innoArgs += "/sDefaultsign=`"$signTool`""
    }

    # Execute Inno Setup
    Write-Host "🚀 Running Inno Setup: $innoSetupPath $innoArgs"
    $process = Start-Process -FilePath $innoSetupPath -ArgumentList $innoArgs -NoNewWindow -Wait -PassThru

    if ($process.ExitCode -eq 0) {
        Write-Host "✅ Inno Setup successfully compiled the installer"
        if ($env:GITHUB_ENV) {
            Write-Output "SETUP_PATH=target/$appSetupName.exe" >> $env:GITHUB_ENV
        }
        $script:buildSuccess = $true
        $script:installerPath = "$env:FANTA_WORKSPACE\target\$appSetupName.exe"
    }
    else {
        Write-Host "❌ Inno Setup failed: $($process.ExitCode)"
        $script:buildSuccess = $false
    }
}

# The license generator, the cargo build and the downloads all work relative to the
# repository root.
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    ParseFantaWorkspace
    # As in script/bundle-mac, the channel compiled in is FANTA_RELEASE_CHANNEL, else RELEASE_CHANNEL.
    $channel = if ($env:FANTA_RELEASE_CHANNEL) {
        $env:FANTA_RELEASE_CHANNEL
    } else {
        (Get-Content -Raw "$env:FANTA_WORKSPACE\crates\fanta\RELEASE_CHANNEL").Trim()
    }
    $env:FANTA_RELEASE_CHANNEL = $channel
    $env:RELEASE_CHANNEL = $channel

    $CargoOutDir = "$env:FANTA_WORKSPACE\target\$target\release"
    $innoDir = "$env:FANTA_WORKSPACE\inno\$Architecture"
    $downloadDir = "$env:FANTA_WORKSPACE\inno\downloads"
    $debugArchive = "$CargoOutDir\fanta-$env:RELEASE_VERSION-$env:FANTA_RELEASE_CHANNEL.dbg.zip"

    CheckEnvironmentVariables
    EnterVsDevShell
    PrepareForBundle
    GenerateLicenses
    BuildFantaAndItsFriends
    SignFantaAndItsFriends
    ZipFantaAndItsFriendsDebug
    if ($Architecture -eq "x86_64") {
        DownloadAMDGpuServices
    }
    DownloadConpty
    CollectFiles
    BuildInstaller

    if ($env:CI) {
        UploadToSentry
    }
}
finally {
    Pop-Location
}

if ($buildSuccess) {
    Write-Output "Build successful"
    if ($Install) {
        Write-Output "Installing Fanta..."
        Start-Process -FilePath $installerPath
    }
    exit 0
}
else {
    Write-Output "Build failed"
    exit 1
}
