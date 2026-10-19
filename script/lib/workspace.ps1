
function ParseFantaWorkspace {
    $metadata = cargo metadata --no-deps --offline | ConvertFrom-Json
    $env:FANTA_WORKSPACE = $metadata.workspace_root
    $env:RELEASE_VERSION = $metadata.packages | Where-Object { $_.name -eq "fanta" } | Select-Object -ExpandProperty version
}
