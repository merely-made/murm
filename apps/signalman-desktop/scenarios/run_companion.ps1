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
    [string]$PeerBinary,
    [string]$TucketBenchConfig,
    [string]$TucketConfig,
    [string]$BenchPython,
    [string]$PeerUsbSerial,
    [switch]$ExchangeAfterReconnect
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
if (!$DesktopBinary) { $DesktopBinary = Join-Path $repo 'apps/signalman-desktop/target/debug/signalman-desktop.exe' }
if (!$PeerBinary) { $PeerBinary = Join-Path $repo 'target/debug/examples/stock_companion_pair.exe' }
$DesktopBinary = (Resolve-Path -LiteralPath $DesktopBinary).Path
$PeerBinary = (Resolve-Path -LiteralPath $PeerBinary).Path
if ($TucketBenchConfig -and $TucketConfig) { throw 'Choose one Tucket authority.' }
$hasTucket = [bool]($TucketBenchConfig -or $TucketConfig)
if ($hasTucket) {
    if ($Protocol -ne 'meshcore' -or !$BenchPython -or !$PeerUsbSerial) { throw 'Tucket bench requires MeshCore, BenchPython and an exact PeerUsbSerial.' }
    $configurationPath = (Resolve-Path -LiteralPath $(if ($TucketConfig) { $TucketConfig } else { $TucketBenchConfig })).Path
    $BenchPython = (Resolve-Path -LiteralPath $BenchPython).Path
    $tucketSettings = Get-Content -LiteralPath $configurationPath -Raw | ConvertFrom-Json
    if ($tucketSettings.realm -ne $Realm -or $tucketSettings.port -ne $Port) { throw 'Tucket config must explicitly match this port and realm.' }
    if ($TucketBenchConfig -and !$tucketSettings.bench_only) { throw 'Seed-file bench mode must be explicit.' }
    if ($TucketConfig -and (!$tucketSettings.device_id -or $tucketSettings.bench_only -or $tucketSettings.private_material)) { throw 'Djinn configuration must name a public device id and no private material.' }
}
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
    RECONNECT_EXCHANGE=''
}
if ($ExchangeAfterReconnect) {
    $tokens.RECONNECT_EXCHANGE = @'
begin companion reconnect exchange
key tab-until data-text-field=message-recipient
input-text @@PEER@@
key tab-until data-text-field=message-draft
input-text @@TAG@@-reconnect
click role:button Queue text
await companion acknowledged
wait 1800
assert snap protocol_ack == 1
await companion received
wait 1800
assert snap incoming == 1
await companion peer acknowledged
wait 600
assert snap peer_protocol_ack == true
capture reconnect-roundtrip
remember companion history
'@
}
$script = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'companion.scn.in') -Raw
if ($hasTucket) {
    $script = $script.Replace('Connect companion', 'Connect Tulle').Replace('Signalman stock @@PROTOCOL@@ companion', 'Signalman Tucket over Tulle against stock @@PROTOCOL@@')
    $tucketSettings.journal = $journal
    $configurationPath = Join-Path $output 'tucket-config.json'
    $tucketSettings | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $configurationPath
    if ($TucketConfig) { $TucketConfig = $configurationPath } else { $TucketBenchConfig = $configurationPath }
}
$script = $script.Replace('@@RECONNECT_EXCHANGE@@', $tokens.RECONNECT_EXCHANGE)
foreach ($token in $tokens.Keys) { $script = $script.Replace("@@$token@@", $tokens[$token]) }
[IO.File]::WriteAllText($fixture, $script)
$envNames = @('SIGNALMAN_COMPANION_SCENARIO','SIGNALMAN_COMPANION_CAPTURE_DIR',
    'SIGNALMAN_COMPANION_RECEIPT','SIGNALMAN_COMPANION_TAG','SIGNALMAN_COMPANION_PROTOCOL',
    'SIGNALMAN_COMPANION_PEER','SIGNALMAN_COMPANION_PEER_RECEIPT','SIGNALMAN_MESSAGE_STORE','SIGNALMAN_AVAILABILITY_SETTINGS',
    'SIGNALMAN_APPEARANCE_SCENARIO','SIGNALMAN_STATION_PORT','SIGNALMAN_RADIO_JOURNAL',
    'SIGNALMAN_MESHTASTIC_PORT','SIGNALMAN_MESHCORE_PORT','SIGNALMAN_APPEARANCE_STORE',
    'SIGNALMAN_THEME_LIBRARY','SIGNALMAN_OBSERVATION_FIXTURES','SIGNALMAN_WIDTH','SIGNALMAN_HEIGHT',
    'SIGNALMAN_TUCKET_BENCH_CONFIG','SIGNALMAN_TUCKET_CONFIG',
    'SIGNALMAN_COMPANION_RECONNECT_PEER_RECEIPT','SIGNALMAN_COMPANION_RECONNECT_READY')
$saved = @{}
foreach ($name in $envNames) { $saved[$name] = [Environment]::GetEnvironmentVariable($name) }
$receipt = [ordered]@{
    schema=1; source=(git -C $repo rev-parse HEAD); protocol=$Protocol; port=$Port; peer_port=$PeerPort
    self_address=$SelfAddress; peer_address=$PeerAddress; realm=$Realm; channel=$Channel; tag=$tag
    desktop_sha256=(Get-FileHash $DesktopBinary -Algorithm SHA256).Hash
    peer_sha256=(Get-FileHash $PeerBinary -Algorithm SHA256).Hash
    scenario_sha256=(Get-FileHash $fixture -Algorithm SHA256).Hash
    deadline_seconds=$DeadlineSeconds; firmware_or_configuration_mutated=$false; result='not_completed'
    exchange_after_reconnect=[bool]$ExchangeAfterReconnect
    carriage=$(if ($TucketConfig) { 'djinn_tucket_over_tulle' } elseif ($TucketBenchConfig) { 'isolated_bench_tucket_over_tulle' } else { 'stock_companion' })
}
$desktop = $null
$peer = $null
$reconnectPeer = $null
try {
    foreach ($name in $envNames) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
    $env:SIGNALMAN_COMPANION_SCENARIO = $fixture
    $env:SIGNALMAN_COMPANION_CAPTURE_DIR = $output
    $env:SIGNALMAN_COMPANION_RECEIPT = Join-Path $output 'scenario.done'
    $env:SIGNALMAN_COMPANION_TAG = $tag
    $env:SIGNALMAN_COMPANION_PROTOCOL = $Protocol
    $env:SIGNALMAN_COMPANION_PEER = $PeerAddress
    $env:SIGNALMAN_COMPANION_PEER_RECEIPT = Join-Path $output 'peer/receipt.json'
    if ($ExchangeAfterReconnect) {
        $env:SIGNALMAN_COMPANION_RECONNECT_PEER_RECEIPT = Join-Path $output 'peer-reconnect/receipt.json'
        $env:SIGNALMAN_COMPANION_RECONNECT_READY = Join-Path $output 'reconnect.ready'
    }
    $env:SIGNALMAN_MESSAGE_STORE = Join-Path $output 'messages.json'
    $env:SIGNALMAN_AVAILABILITY_SETTINGS = Join-Path $output 'availability.json'
    $env:SIGNALMAN_APPEARANCE_STORE = Join-Path $output 'appearance.json'
    $env:SIGNALMAN_THEME_LIBRARY = Join-Path $output 'themes'
    $env:SIGNALMAN_WIDTH = "$Width"
    $env:SIGNALMAN_HEIGHT = "$Height"
    $peerArgs = "peer $Protocol $PeerPort $Realm `"$(Join-Path $output 'peer')`" $tag $SelfAddress $Channel"
    $deadline = [DateTime]::UtcNow.AddSeconds($DeadlineSeconds)
    if ($TucketBenchConfig) { $env:SIGNALMAN_TUCKET_BENCH_CONFIG = $TucketBenchConfig }
    if ($TucketConfig) { $env:SIGNALMAN_TUCKET_CONFIG = $TucketConfig }
    $desktop = Start-Process -FilePath $DesktopBinary -PassThru -WorkingDirectory $repo `
        -RedirectStandardOutput (Join-Path $output 'desktop.stdout.log') -RedirectStandardError (Join-Path $output 'desktop.stderr.log')
    if ($hasTucket) {
        # Wait for confirmed host carriage, then put a genuine stock advert on
        # air. The stock helper owns the port afterward and drains stored text.
        while (!(Test-Path -LiteralPath $journal)) {
            $desktop.Refresh()
            if ($desktop.HasExited) { throw 'Desktop exited before opening its Tucket journal.' }
            if ([DateTime]::UtcNow -ge $deadline) { throw 'Tucket connection deadline exceeded.' }
            Start-Sleep -Milliseconds 250
        }
        & $BenchPython -B (Join-Path $repo 'testing/stock_tucket_bench.py') advertise --port $PeerPort --usb-serial $PeerUsbSerial --output (Join-Path $output 'stock-advertise.json')
        if ($LASTEXITCODE -ne 0) { throw 'Stock peer advertisement failed.' }
        Start-Sleep -Milliseconds 500
    }
    $peer = Start-Process -FilePath $PeerBinary -ArgumentList $peerArgs -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $output 'peer.stdout.log') -RedirectStandardError (Join-Path $output 'peer.stderr.log')
    $receipt.peer_pid = $peer.Id
    $receipt.desktop_pid = $desktop.Id
    while (!$desktop.HasExited -or !$peer.HasExited -or ($reconnectPeer -and !$reconnectPeer.HasExited)) {
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Absolute acceptance deadline exceeded.' }
        if ($desktop.HasExited -and $desktop.ExitCode -ne 0) { throw "Desktop exited $($desktop.ExitCode)." }
        if ($peer.HasExited -and $peer.ExitCode -ne 0) { throw "Peer exited $($peer.ExitCode)." }
        if ($reconnectPeer -and $reconnectPeer.HasExited -and $reconnectPeer.ExitCode -ne 0) { throw "Reconnect peer exited $($reconnectPeer.ExitCode)." }
        if ($ExchangeAfterReconnect -and !$reconnectPeer -and $peer.HasExited -and (Test-Path -LiteralPath $env:SIGNALMAN_COMPANION_RECONNECT_READY)) {
            $first = Get-Content -LiteralPath (Join-Path $output 'peer/receipt.json') -Raw | ConvertFrom-Json
            if ($first.result -ne 'passed') { throw 'First peer receipt did not pass before reconnect.' }
            if ($hasTucket) {
                & $BenchPython -B (Join-Path $repo 'testing/stock_tucket_bench.py') advertise --port $PeerPort --usb-serial $PeerUsbSerial --output (Join-Path $output 'stock-reconnect-advertise.json')
                if ($LASTEXITCODE -ne 0) { throw 'Stock reconnect advertisement failed.' }
                Start-Sleep -Milliseconds 500
            }
            $reconnectArgs = "peer $Protocol $PeerPort $Realm `"$(Join-Path $output 'peer-reconnect')`" $tag-reconnect $SelfAddress $Channel"
            $reconnectPeer = Start-Process -FilePath $PeerBinary -ArgumentList $reconnectArgs -PassThru -WindowStyle Hidden `
                -RedirectStandardOutput (Join-Path $output 'peer-reconnect.stdout.log') -RedirectStandardError (Join-Path $output 'peer-reconnect.stderr.log')
            $receipt.reconnect_peer_pid = $reconnectPeer.Id
        }
        Start-Sleep -Milliseconds 250
        $desktop.Refresh()
        $peer.Refresh()
        if ($reconnectPeer) { $reconnectPeer.Refresh() }
    }
    $receipt.desktop_exit = $desktop.ExitCode
    $receipt.peer_exit = $peer.ExitCode
    if ($desktop.ExitCode -ne 0 -or $peer.ExitCode -ne 0) { throw 'A companion process failed.' }
    if (!(Test-Path -LiteralPath $env:SIGNALMAN_COMPANION_RECEIPT)) { throw 'Desktop omitted its scenario receipt.' }
    $peerReceipt = Get-Content -LiteralPath (Join-Path $output 'peer/receipt.json') -Raw | ConvertFrom-Json
    if ($peerReceipt.result -ne 'passed') { throw 'Peer receipt did not pass.' }
    if ($ExchangeAfterReconnect) {
        if (!$reconnectPeer -or !$reconnectPeer.HasExited -or $reconnectPeer.ExitCode -ne 0) { throw 'Reconnect peer omitted successful completion.' }
        $second = Get-Content -LiteralPath (Join-Path $output 'peer-reconnect/receipt.json') -Raw | ConvertFrom-Json
        if ($second.result -ne 'passed') { throw 'Reconnect peer receipt did not pass.' }
        $receipt.reconnect_peer_exit = $reconnectPeer.ExitCode
    }
    if ((Get-FileHash $DesktopBinary -Algorithm SHA256).Hash -ne $receipt.desktop_sha256 -or
        (Get-FileHash $PeerBinary -Algorithm SHA256).Hash -ne $receipt.peer_sha256) { throw 'A bench executable changed during the native run.' }
    $receipt.result = 'completed_pending_visual_review'
} catch {
    $receipt.result = 'failed'
    $receipt.error = $_.Exception.Message
    throw
} finally {
    foreach ($process in @($desktop,$peer,$reconnectPeer)) {
        if ($process -and !$process.HasExited) { Stop-Process -Id $process.Id; $process.WaitForExit() }
    }
    foreach ($name in $envNames) {
        if ($null -eq $saved[$name]) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
        else { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    }
    $receipt.closed_utc = [DateTime]::UtcNow.ToString('o')
    $receipt | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $output 'run.json')
}
