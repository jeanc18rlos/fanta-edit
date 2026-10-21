$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$CARGO_ABOUT_VERSION="0.8.2"
$outputFile=$args[0] ? $args[0] : "$(Get-Location)/assets/licenses.md"
$templateFile="script/licenses/template.md.hbs"
# Some crates have features whose names differ only in case (USB and usb), which
# ConvertFrom-Json rejects unless it builds case-sensitive hashtables.
$metadata = cargo metadata --format-version 1 --locked | ConvertFrom-Json -AsHashtable
if ($LASTEXITCODE -ne 0) { throw "Cargo metadata failed" }
$assetPackage = $metadata.packages | Where-Object { $_.name -eq 'fanta-gpui-zed-ui-assets' } | Select-Object -First 1
if (-not $assetPackage) { throw "fanta-gpui-zed-ui-assets is missing from Cargo metadata" }
$assetDirectory = Join-Path (Split-Path $assetPackage.manifest_path) 'assets'

New-Item -Path "$outputFile" -ItemType File -Value "" -Force

@(
    "# ###### THEME LICENSES ######\n"
    Get-Content (Join-Path $assetDirectory 'themes/LICENSES')
    "\n# ###### ICON LICENSES ######\n"
    Get-Content (Join-Path $assetDirectory 'icons/LICENSES')
    "\n# ###### CODE LICENSES ######\n"
) | Add-Content -Path $outputFile

$needsInstall = $false
try {
    $versionOutput = cargo about --version
    if (-not ($versionOutput -match "cargo-about $CARGO_ABOUT_VERSION")) {
        $needsInstall = $true
    } else {
        Write-Host "cargo-about@$CARGO_ABOUT_VERSION is already installed"
    }
} catch {
    $needsInstall = $true
}

if ($needsInstall) {
    Write-Host "Installing cargo-about@$CARGO_ABOUT_VERSION..."
    cargo install "cargo-about@$CARGO_ABOUT_VERSION"
}

Write-Host "Generating cargo licenses"

# As in script/generate-licenses: a crate without an accepted license fails the build unless
# ALLOW_MISSING_LICENSES is set.
$failFlag = $env:ALLOW_MISSING_LICENSES ? "" : "--fail"
# cargo-about's -o replaces the file, which would drop the theme and icon licenses above.
$codeLicenses = New-TemporaryFile
$aboutArgs = @('about', 'generate', $failFlag, '-c', 'script/licenses/fanta-licenses.toml', $templateFile, '-o', $codeLicenses.FullName) | Where-Object { $_ }
cargo @aboutArgs
Get-Content $codeLicenses.FullName | Add-Content -Path $outputFile
Remove-Item $codeLicenses.FullName

Write-Host "Applying replacements"
$replacements = @{
    '&quot;' = '"'
    '&#x27;' = "'"
    '&#x3D;' = '='
    '&#x60;' = '`'
    '&lt;'   = '<'
    '&gt;'   = '>'
}
$content = Get-Content $outputFile
foreach ($find in $replacements.keys) {
    $content = $content -replace $find, $replacements[$find]
}
$content | Set-Content $outputFile

Write-Host "generate-licenses completed. See $outputFile"
