# Build the firmware images a package index templates and stage a verified catalog.
#
# protocol_parity D4: the repository keeps package templates, never built images.
# This script builds each templated image from HEAD, then `linkboy stage-catalog`
# completes every template with the image's length, digest, write layout and the
# source revision, copies untemplated (third-party) packages as they are, and
# verifies the whole staged catalog. Nothing is downloaded.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Index,

    # The stage's firmware root; the catalog lands in its packages directory.
    [Parameter(Mandatory = $true)]
    [string]$Destination,

    [Parameter(Mandatory = $true)]
    [string]$LinkboyBinary,

    # Needed only when the index templates the Heltec V4 package.
    [string]$Espflash,

    # Build from a tree with uncommitted changes; the manifest still names HEAD.
    [switch]$AllowDirty,

    # Pass --ignore-rust-version to the ESP build when the installed esp toolchain
    # is older than the workspace's declared rust-version.
    [switch]$IgnoreRustVersion
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$indexPath = (Resolve-Path -LiteralPath $Index).Path
$linkboyPath = (Resolve-Path -LiteralPath $LinkboyBinary).Path

function Invoke-Checked {
    param([string]$Program, [string[]]$Arguments)
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

$destinationFull = [System.IO.Path]::GetFullPath($Destination)
if ($destinationFull.StartsWith($repoRoot + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Stage outside the repository; built images are not kept in it: $destinationFull"
}

Push-Location $repoRoot
try {
    $dirty = git status --porcelain --untracked-files=no
    if ($dirty -and -not $AllowDirty) {
        throw "The working tree has uncommitted changes; commit them or pass -AllowDirty."
    }
    $revision = (git rev-parse HEAD).Trim()
    $commitTime = (git log -1 --format=%ct HEAD).Trim()
    $targetDir = (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
    $work = Join-Path $destinationFull '..\firmware-build'
    if (Test-Path -LiteralPath $work) {
        throw "Build scratch already exists: $work"
    }
    New-Item -ItemType Directory -Path $work | Out-Null

    $templated = (Get-Content -LiteralPath $indexPath -Raw) |
        Select-String -Pattern 'manifest = "([^"]+)\.toml"' -AllMatches |
        ForEach-Object { $_.Matches } |
        ForEach-Object { $_.Groups[1].Value } |
        Where-Object { Test-Path -LiteralPath (Join-Path (Split-Path $indexPath) "$_.template.toml") }

    $stageArgs = @('stage-catalog', $indexPath, (Join-Path $destinationFull 'packages'), $revision)
    $env:RETINUE_FIRMWARE_REVISION = $revision
    foreach ($manifest in $templated) {
        switch ($manifest) {
            'heltec-v4-current' {
                if (-not $Espflash) { throw 'The V4 package needs -Espflash to report its write size.' }
                $espflashVersion = & $Espflash --version
                if ($espflashVersion -notmatch '\b4\.5\.0\b') {
                    throw "The V4 template names espflash 4.5.0; found $espflashVersion"
                }
                # The app descriptor records its build time; the commit time makes it repeat.
                $env:SOURCE_DATE_EPOCH = $commitTime
                $build = @('+esp', 'build', '-p', 'tulle-heltec-v4-phy', '--release',
                    '--target', 'xtensa-esp32s3-none-elf', '-Zbuild-std=core')
                if ($IgnoreRustVersion) { $build += '--ignore-rust-version' }
                Invoke-Checked 'cargo' $build
                Remove-Item Env:SOURCE_DATE_EPOCH
                $elf = Join-Path $targetDir 'xtensa-esp32s3-none-elf\release\tulle-heltec-v4-phy'
                $image = Join-Path $work 'tulle-heltec-v4-phy.app.bin'
                Invoke-Checked $Espflash @('save-image', '--chip', 'esp32s3', $elf, $image)
                # espflash writes the application at 0x10000, after bootloader and partition table.
                $writeBytes = 0x10000 + (Get-Item -LiteralPath $image).Length
                $stageArgs += @('--payload', 'retinue.heltec-v4', $elf,
                    '--write-bytes', 'retinue.heltec-v4', "$writeBytes")
            }
            't114-current' {
                Invoke-Checked 'cargo' @('build', '-p', 'tulle-t114-phy', '--release',
                    '--target', 'thumbv7em-none-eabihf')
                $elf = Join-Path $targetDir 'thumbv7em-none-eabihf\release\tulle-t114-phy'
                $bin = Join-Path $work 'tulle-t114-phy.bin'
                $uf2 = Join-Path $work 'tulle-t114-phy.uf2'
                Invoke-Checked 'rust-objcopy' @('-O', 'binary', $elf, $bin)
                Invoke-Checked $linkboyPath @('make-uf2', $bin, $uf2, '0x26000', '0xADA52840')
                $stageArgs += @('--payload', 'retinue.t114', $uf2)
            }
            default { throw "No build recipe for templated package $manifest" }
        }
    }
    Remove-Item Env:RETINUE_FIRMWARE_REVISION
    Invoke-Checked $linkboyPath $stageArgs
    Remove-Item -LiteralPath $work -Recurse
}
finally {
    Pop-Location
}
