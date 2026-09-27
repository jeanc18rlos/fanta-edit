$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$CARGO_ABOUT_VERSION="0.8.2"
$outputFile=$args[0] ? $args[0] : "$(Get-Location)/assets/licenses.md"
$templateFile="script/licenses/template.md.hbs"
$metadata = cargo metadata --format-version 1 --locked | ConvertFrom-Json
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

$failFlag = $env:ALLOW_MISSING_LICENSES ? "--fail" : ""
$args = @('about', 'generate', $failFlag, '-c', 'script/licenses/zed-licenses.toml', $templateFile, '-o', $outputFile) | Where-Object { $_ }
cargo @args

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
