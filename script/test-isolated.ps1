# Runs one crate's library tests. A native crash, for example inside Skia, ends the
# whole test process and hides every test after it. So after a failure, each test runs
# again in its own process, and the script lists every test that fails.
#
#   .\script\test-isolated.ps1 -Package fanta-render

param(
    [Parameter(Mandatory = $true)]
    [string]$Package
)

$ErrorActionPreference = 'Continue'

cargo test --locked --lib -p $Package
if ($LASTEXITCODE -eq 0) {
    exit 0
}

$target = $Package -replace '-', '_'
$artifact = cargo test --locked --lib -p $Package --no-run --message-format=json |
    ConvertFrom-Json |
    Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq $target -and $_.profile.test } |
    Select-Object -Last 1
if (-not $artifact) {
    Write-Output "The tests of $Package did not build."
    exit 1
}

$binary = $artifact.executable
# Match `cargo test`, which runs tests from the package directory.
$env:CARGO_MANIFEST_DIR = Split-Path -Parent $artifact.manifest_path
Set-Location $env:CARGO_MANIFEST_DIR

$failed = @()
foreach ($line in & $binary --list --format terse) {
    if ($line -notmatch '^(.+): test$') {
        continue
    }
    $name = $Matches[1]
    & $binary --exact $name *> $null
    if ($LASTEXITCODE -ne 0) {
        $failed += '{0} (exit code 0x{1:X8})' -f $name, $LASTEXITCODE
    }
}

Write-Output "Tests of $Package that fail in their own process: $($failed.Count)"
$failed | ForEach-Object { Write-Output "  $_" }
exit 1
