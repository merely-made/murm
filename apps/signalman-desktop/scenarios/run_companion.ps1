# Finite attended stock-radio acceptance. Both radios must already be configured
# compatibly, with contacts exchanged. This runner never flashes/configures them.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('meshtastic', 'meshcore')][string]$Protocol,
    [Parameter(Mandatory)][ValidatePattern('^COM[0-9]+$')][string]$Port,
    [Parameter(Mandatory)][ValidatePattern('^COM[0-9]+$')][string]$PeerPort,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-f]+$')][string]$SelfAddress,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-f]+$')][string]$PeerAddress,
    [Parameter(Mandatory)][ValidatePattern('^[a-zA-Z0-9_-]{1,128}$')][string]$Realm,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateRange(0,7)][int]$Channel = 0,
    [ValidateRange(30,180)][int]$DeadlineSeconds = 180,
    [ValidateRange(640,2560)][int]$Width = 1200,
    [ValidateRange(480,1600)][int]$Height = 1100,
    [ValidateRange(0.5,2.0)][double]$Zoom = 1.0,
    [string]$DesktopBinary,
    [string]$PeerBinary
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
if (!$DesktopBinary) { $DesktopBinary = Join-Path $repo 'apps/signalman-desktop/target/debug/signalman-desktop.exe' }
if (!$PeerBinary) { $PeerBinary = Join-Path $repo 'target/debug/examples/stock_companion_pair.exe' }
$DesktopBinary = (Resolve-Path -LiteralPath $DesktopBinary).Path
$PeerBinary = (Resolve-Path -LiteralPath $PeerBinary).Path
$length = if ($Protocol -eq 'meshtastic') { 8 } else { 64 }
if ($Port -eq $PeerPort -or $SelfAddress -eq $PeerAddress -or
    $SelfAddress.Length -ne $length -or $PeerAddress.Length -ne $length) {
    throw 'Require distinct identified ports and full public addresses for the selected protocol.'
}
$output = [IO.Path]::GetFullPath($OutputDirectory)
if ($output -match '[\r\n"]') { throw 'Output path cannot contain quotes or line breaks.' }
if (Test-Path -LiteralPath $output) { throw 'Require a fresh output directory; preserve earlier trials.' }
New-Item -ItemType Directory -Path $output | Out-Null
$tag = 'signalman-ui-' + [guid]::NewGuid().ToString('N')
$journal = Join-Path $output 'desktop-journal.json'
$fixture = Join-Path $output 'companion.scn'
$tokens = @{
    PROTOCOL=$Protocol; LABEL=$(if ($Protocol -eq 'meshtastic') { 'Meshtastic' } else { 'MeshCore' })
    PORT=$Port; REALM=$Realm; JOURNAL=$journal; CHANNEL="$Channel"; PEER=$PeerAddress; TAG=$tag
    ZOOM=$Zoom.ToString([Globalization.CultureInfo]::InvariantCulture)
}
$script = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'companion.scn.in') -Raw
foreach ($token in $tokens.Keys) { $script = $script.Replace("@@$token@@", $tokens[$token]) }
[IO.File]::WriteAllText($fixture, $script)
$envNames = @('SIGNALMAN_COMPANION_SCENARIO','SIGNALMAN_COMPANION_CAPTURE_DIR',
    'SIGNALMAN_COMPANION_RECEIPT','SIGNALMAN_COMPANION_TAG','SIGNALMAN_COMPANION_PROTOCOL',
    'SIGNALMAN_COMPANION_PEER','SIGNALMAN_MESSAGE_STORE','SIGNALMAN_AVAILABILITY_SETTINGS',
    'SIGNALMAN_APPEARANCE_SCENARIO','SIGNALMAN_STATION_PORT','SIGNALMAN_RADIO_JOURNAL',
    'SIGNALMAN_MESHTASTIC_PORT','SIGNALMAN_MESHCORE_PORT','SIGNALMAN_APPEARANCE_STORE',
    'SIGNALMAN_THEME_LIBRARY','SIGNALMAN_OBSERVATION_FIXTURES','SIGNALMAN_WIDTH','SIGNALMAN_HEIGHT')
$saved = @{}
foreach ($name in $envNames) { $saved[$name] = [Environment]::GetEnvironmentVariable($name) }
$receipt = [ordered]@{
    schema=1; source=(git -C $repo rev-parse HEAD); protocol=$Protocol; port=$Port; peer_port=$PeerPort
    self_address=$SelfAddress; peer_address=$PeerAddress; realm=$Realm; channel=$Channel; tag=$tag
    desktop_sha256=(Get-FileHash $DesktopBinary -Algorithm SHA256).Hash
    peer_sha256=(Get-FileHash $PeerBinary -Algorithm SHA256).Hash
    scenario_sha256=(Get-FileHash $fixture -Algorithm SHA256).Hash
    deadline_seconds=$DeadlineSeconds; firmware_or_configuration_mutated=$false; result='not_completed'
}
$desktop = $null
$peer = $null
try {
    foreach ($name in $envNames) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
    $env:SIGNALMAN_COMPANION_SCENARIO = $fixture
    $env:SIGNALMAN_COMPANION_CAPTURE_DIR = $output
    $env:SIGNALMAN_COMPANION_RECEIPT = Join-Path $output 'scenario.done'
    $env:SIGNALMAN_COMPANION_TAG = $tag
    $env:SIGNALMAN_COMPANION_PROTOCOL = $Protocol
    $env:SIGNALMAN_COMPANION_PEER = $PeerAddress
    $env:SIGNALMAN_MESSAGE_STORE = Join-Path $output 'messages.json'
    $env:SIGNALMAN_AVAILABILITY_SETTINGS = Join-Path $output 'availability.json'
    $env:SIGNALMAN_APPEARANCE_STORE = Join-Path $output 'appearance.json'
    $env:SIGNALMAN_THEME_LIBRARY = Join-Path $output 'themes'
    $env:SIGNALMAN_WIDTH = "$Width"
    $env:SIGNALMAN_HEIGHT = "$Height"
    $peerArgs = "peer $Protocol $PeerPort $Realm `"$(Join-Path $output 'peer')`" $tag $SelfAddress $Channel"
    $deadline = [DateTime]::UtcNow.AddSeconds($DeadlineSeconds)
    $peer = Start-Process -FilePath $PeerBinary -ArgumentList $peerArgs -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $output 'peer.stdout.log') -RedirectStandardError (Join-Path $output 'peer.stderr.log')
    $desktop = Start-Process -FilePath $DesktopBinary -PassThru -WorkingDirectory $repo `
        -RedirectStandardOutput (Join-Path $output 'desktop.stdout.log') -RedirectStandardError (Join-Path $output 'desktop.stderr.log')
    $receipt.peer_pid = $peer.Id
    $receipt.desktop_pid = $desktop.Id
    while (!$desktop.HasExited -or !$peer.HasExited) {
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Absolute acceptance deadline exceeded.' }
        if ($desktop.HasExited -and $desktop.ExitCode -ne 0) { throw "Desktop exited $($desktop.ExitCode)." }
        if ($peer.HasExited -and $peer.ExitCode -ne 0) { throw "Peer exited $($peer.ExitCode)." }
        Start-Sleep -Milliseconds 250
        $desktop.Refresh()
        $peer.Refresh()
    }
    $receipt.desktop_exit = $desktop.ExitCode
    $receipt.peer_exit = $peer.ExitCode
    if ($desktop.ExitCode -ne 0 -or $peer.ExitCode -ne 0) { throw 'A companion process failed.' }
    if (!(Test-Path -LiteralPath $env:SIGNALMAN_COMPANION_RECEIPT)) { throw 'Desktop omitted its scenario receipt.' }
    $peerReceipt = Get-Content -LiteralPath (Join-Path $output 'peer/receipt.json') -Raw | ConvertFrom-Json
    if ($peerReceipt.result -ne 'passed') { throw 'Peer receipt did not pass.' }
    $receipt.result = 'completed_pending_visual_review'
} catch {
    $receipt.result = 'failed'
    $receipt.error = $_.Exception.Message
    throw
} finally {
    foreach ($process in @($desktop,$peer)) {
        if ($process -and !$process.HasExited) { Stop-Process -Id $process.Id; $process.WaitForExit() }
    }
    foreach ($name in $envNames) {
        if ($null -eq $saved[$name]) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
        else { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    }
    $receipt.closed_utc = [DateTime]::UtcNow.ToString('o')
    $receipt | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $output 'run.json')
}
