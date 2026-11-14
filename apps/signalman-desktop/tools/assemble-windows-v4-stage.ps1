[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$DesktopBinary,

    [Parameter(Mandatory = $true)]
    [string]$LinkboyBinary,

    [Parameter(Mandatory = $true)]
    [string]$Espflash,

    [Parameter(Mandatory = $true)]
    [string]$Destination,

    # Forwarded to stage-firmware.ps1.
    [switch]$AllowDirty,
    [switch]$IgnoreRustVersion
)

$ErrorActionPreference = 'Stop'
$expectedEspflashSha256 = '0cc03364c70a86325236f18ad1aaed17eedf267d89312c0cdabe4964f5cb758e'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$binaryPath = (Resolve-Path -LiteralPath $DesktopBinary).Path
$linkboyPath = (Resolve-Path -LiteralPath $LinkboyBinary).Path
$espflashPath = (Resolve-Path -LiteralPath $Espflash).Path

if (Test-Path -LiteralPath $Destination) {
    throw "Destination already exists: $Destination"
}

$actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $espflashPath).Hash.ToLowerInvariant()
if ($actualHash -ne $expectedEspflashSha256) {
    throw "espflash digest mismatch: expected $expectedEspflashSha256, found $actualHash"
}

$stageRoot = New-Item -ItemType Directory -Path $Destination
$helperRoot = New-Item -ItemType Directory -Path (Join-Path $stageRoot.FullName 'helpers\windows-x86_64') -Force
$firmwareRoot = Join-Path $stageRoot.FullName 'firmware'

Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $stageRoot.FullName 'signalman-desktop.exe')
Copy-Item -LiteralPath $linkboyPath -Destination (Join-Path $stageRoot.FullName 'linkboy.exe')
Copy-Item -LiteralPath $espflashPath -Destination (Join-Path $helperRoot.FullName 'espflash.exe')
# Builds the V4 image from HEAD and stages a catalog that verifies (protocol_parity D4).
& (Join-Path $PSScriptRoot 'stage-firmware.ps1') -Index (Join-Path $repoRoot 'firmware\packages\windows-v4-staging-index.toml') `
    -Destination $firmwareRoot -LinkboyBinary $linkboyPath -Espflash $espflashPath `
    -AllowDirty:$AllowDirty -IgnoreRustVersion:$IgnoreRustVersion
Copy-Item -LiteralPath (Join-Path $repoRoot 'firmware\packages\heltec-v4-current-recovery.md') -Destination (Join-Path $firmwareRoot 'packages')
Copy-Item -LiteralPath (Join-Path $repoRoot 'apps\linkboy\NOTICES.md') -Destination (Join-Path $stageRoot.FullName 'NOTICES.md')
Copy-Item -LiteralPath (Join-Path $repoRoot 'LICENSE') -Destination (Join-Path $stageRoot.FullName 'LICENSE')

@{
    schema = 'retinue.windows-v4-stage/v1'
    helper = @{
        program = 'espflash'
        version = '4.5.0'
        sha256 = $actualHash
        license = 'MIT OR Apache-2.0'
        source = 'https://github.com/esp-rs/espflash'
    }
    recovery_cli = 'linkboy.exe'
    catalog = 'firmware/packages/index.toml'
    packages = @('retinue.heltec-v4')
    excluded = @('retinue.t114', 'meshtastic.heltec-mesh-node-t114')
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $stageRoot.FullName 'stage.json') -Encoding utf8NoBOM

Write-Output "assembled $($stageRoot.FullName)"
