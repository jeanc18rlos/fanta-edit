$ErrorActionPreference = "Stop"

Write-Host "Your PATH entries:"
$env:Path -split ";" | ForEach-Object { Write-Host "  $_" }

$needAddWorkspace = $false
if ($args -notcontains "-p" -and $args -notcontains "--package")
{
    $needAddWorkspace = $true
}

# https://stackoverflow.com/questions/41324882/how-to-run-a-powershell-script-with-verbose-output/70020655#70020655
# Set-PSDebug -Trace 2

if ($env:CARGO)
{
    $Cargo = $env:CARGO
} elseif (Get-Command "cargo" -ErrorAction SilentlyContinue)
{
    $Cargo = "cargo"
} else
{
    Write-Error "Could not find cargo in path." -ErrorAction Stop
}

# Skia's Metal backend compiles only against Apple's SDK, so --all-features
# cannot build on Windows. Every other feature of the linted packages is
# enabled instead. script/clippy carries the same list.
$appleOnlyFeatures = @("fanta-render/gpu", "fanta-render/metal")

$packages = @()
for ($index = 0; $index -lt $args.Count - 1; $index++)
{
    if ($args[$index] -eq "-p" -or $args[$index] -eq "--package")
    {
        $packages += $args[$index + 1]
    }
}

$metadataArguments = @("metadata", "--no-deps", "--format-version", "1")
# Without --locked here, metadata would repair a stale lockfile and the
# --locked clippy run after it would pass.
if ($args -contains "--locked")
{
    $metadataArguments += "--locked"
}
$metadataJson = & $Cargo @metadataArguments | Out-String
if ($LASTEXITCODE -ne 0)
{
    Write-Error "cargo metadata failed." -ErrorAction Stop
}
$metadata = $metadataJson | ConvertFrom-Json

$features = @()
foreach ($package in $metadata.packages)
{
    if ($needAddWorkspace -or $packages -contains $package.name)
    {
        foreach ($feature in $package.features.PSObject.Properties.Name)
        {
            $qualified = "$($package.name)/$feature"
            if ($appleOnlyFeatures -notcontains $qualified)
            {
                $features += $qualified
            }
        }
    }
}
# Windows PowerShell drops an empty argument, which would leave --features
# without a value.
$featureArguments = @()
if ($features.Count -gt 0)
{
    $featureArguments = @("--features", ($features -join ","))
}

if ($needAddWorkspace)
{
    & $Cargo clippy @args --workspace --release --all-targets @featureArguments -- --deny warnings
} else
{
    & $Cargo clippy @args --release --all-targets @featureArguments -- --deny warnings
}
