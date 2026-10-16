# Finite, explicitly isolated Tucket/Tulle native bench with physical restoration.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$BenchConfig,
    [Parameter(Mandatory)][string]$SelfAddress,
    [Parameter(Mandatory)][string]$DutUsbSerial,
    [Parameter(Mandatory)][string]$PeerPort,
    [Parameter(Mandatory)][string]$PeerUsbSerial,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Python,
    [string]$PublicCard,
    [string]$DutBaseline,
    [ValidateRange(-9,20)][int]$StockPowerDbm = 0,
    [string]$RawProbe,
    [string]$RawFloodText,
    [switch]$RawDirectFirst,
    [switch]$DjinnCustody,
    [ValidateRange(0.5,2.0)][double]$Zoom = 0.75
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$BenchConfig = (Resolve-Path -LiteralPath $BenchConfig).Path
$Python = (Resolve-Path -LiteralPath $Python).Path
$config = Get-Content -LiteralPath $BenchConfig -Raw | ConvertFrom-Json
if ($config.realm -notlike 'bench-*' -or $config.port -eq $PeerPort -or $DutUsbSerial -eq $PeerUsbSerial -or $SelfAddress -notmatch '^[0-9a-f]{64}$') { throw 'Require an explicit bench realm, full public key and distinct identified boards.' }
if ($DjinnCustody) {
    if (!$config.device_id -or $config.bench_only -or $config.private_material -or $RawProbe) { throw 'Djinn mode requires only public identity configuration and the real desktop worker.' }
} elseif (!$config.bench_only) { throw 'Seed-file mode must be explicit.' }
$output = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $output) { throw 'Choose a fresh evidence directory.' }
$artifacts = Join-Path $repo 'validation/results/sennet-tucket-radios/artifacts'
$helper = Join-Path $artifacts 'espflash.exe'
$stockImage = Join-Path $artifacts 'heltec_v4_companion_radio_usb-v1.17.1-d929643-merged.bin'
$tulleImage = Join-Path $repo 'target/xtensa-esp32s3-none-elf/release/tulle-heltec-v4-phy'
$stockPin = Get-Content (Join-Path $repo 'validation/results/stock-baseline-20261014/meshcore-flash.json') -Raw | ConvertFrom-Json
$tullePin = Get-Content (Join-Path $repo 'validation/results/stock-baseline-20261014/tulle-restore.json') -Raw | ConvertFrom-Json
if ((Get-FileHash $helper -Algorithm SHA256).Hash -ne $stockPin.helper_sha256 -or $stockPin.helper_sha256 -ne $tullePin.helper_sha256 -or
    (Get-FileHash $stockImage -Algorithm SHA256).Hash -ne $stockPin.image_sha256 -or
    (Get-FileHash $tulleImage -Algorithm SHA256).Hash -ne $tullePin.image_sha256) { throw 'Pinned helper or stock/restore image changed.' }
New-Item -ItemType Directory -Path $output | Out-Null
$probe = Join-Path $repo 'testing/stock_tucket_bench.py'
$report = [ordered]@{schema=1;bench_only=$true;source=(git -C $repo rev-parse HEAD);result='not_completed';dut_port=$config.port;peer_port=$PeerPort;dut_usb_serial=$DutUsbSerial;peer_usb_serial=$PeerUsbSerial;helper_sha256=$stockPin.helper_sha256;stock_image_sha256=$stockPin.image_sha256;tulle_image_sha256=$tullePin.image_sha256;restoration=@{}}
$report.tucket_tx_power_dbm = $config.tx_power_dbm
$report.stock_tx_power_dbm = $StockPowerDbm
$report.authority = $(if ($DjinnCustody) { 'djinn' } else { 'isolated_bench_seed_file' })
$report.opened_utc = [DateTime]::UtcNow.ToString('o')
$report.bench_config_sha256 = (Get-FileHash $BenchConfig -Algorithm SHA256).Hash.ToLowerInvariant()
$report.self_public_key = $SelfAddress
if ($PublicCard) { $report.public_card_sha256 = (Get-FileHash -LiteralPath $PublicCard).Hash.ToLowerInvariant() }
$report.source_sha256 = @{}
foreach ($path in @('Cargo.toml','apps/signalman/Cargo.toml','apps/signalman/src/radio_comms.rs','apps/signalman/src/radio_comms/resident.rs','apps/signalman/src/radio_comms/carriage.rs',
    'apps/signalman-desktop/Cargo.toml','apps/signalman-desktop/Cargo.lock','apps/signalman-desktop/src/main.rs',
    'apps/signalman-desktop/src/resident.rs','apps/signalman-desktop/src/resident/bench.rs','apps/signalman-desktop/src/resident/djinn.rs','apps/signalman-desktop/examples/tucket_contact_card.rs','apps/signalman-desktop/src/state/messages.rs',
    'apps/signalman-desktop/src/companion_scenario.rs','apps/signalman-desktop/src/views/messages.rs',
    'apps/signalman-desktop/scenarios/run_companion.ps1','crates/tucket/src/instance.rs',
    'crates/tucket/examples/meshcore_headed/main.rs','crates/tucket/examples/meshcore_headed/companion.rs',
    'crates/tucket/examples/meshcore_headed/radio.rs','testing/stock_tucket_bench.py','testing/run_tucket_desktop.ps1')) {
    $report.source_sha256[$path] = (Get-FileHash (Join-Path $repo $path) -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Invoke-Probe([string[]]$Arguments) {
    & $Python -B $probe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Bench probe failed: $($Arguments[0])" }
}
$peerTouched = $false
$dutStarted = $false
$failure = $null
try {
    Invoke-Probe @('status','--port',$config.port,'--usb-serial',$DutUsbSerial,'--output',(Join-Path $output 'dut-before.json'))
    Invoke-Probe @('status','--port',$PeerPort,'--usb-serial',$PeerUsbSerial,'--output',(Join-Path $output 'peer-before.json'))
    $before = Get-Content (Join-Path $output 'dut-before.json') -Raw | ConvertFrom-Json
    if ($before.channel -notmatch 'channel=modem') { throw 'DUT must already use the Tulle modem channel.' }
    $snapshotArguments = @('snapshot','--port',$config.port,'--usb-serial',$DutUsbSerial,'--output',(Join-Path $output 'dut-profile.json'))
    if ($DutBaseline) { $snapshotArguments += @('--baseline',(Resolve-Path -LiteralPath $DutBaseline).Path) }
    Invoke-Probe $snapshotArguments
    $peerTouched = $true
    & $helper write-bin --port $PeerPort --chip esp32s3 --before usb-reset --baud 921600 --non-interactive --skip-update-check 0 $stockImage *> (Join-Path $output 'stock-flash.log')
    if ($LASTEXITCODE -ne 0) { throw 'Stock firmware transfer failed.' }
    Start-Sleep -Seconds 2
    $setupArguments = @('stock-setup','--port',$PeerPort,'--usb-serial',$PeerUsbSerial,'--peer-key',$SelfAddress,'--frequency-khz',"$([int]($config.frequency_hz/1000))",'--bandwidth-hz',"$($config.bandwidth_hz)",'--sf',"$($config.spreading_factor)",'--cr',"$($config.coding_rate_denominator)",'--power-dbm',"$StockPowerDbm",'--output',(Join-Path $output 'stock-setup.json'))
    if ($PublicCard) { $setupArguments += @('--public-card',(Resolve-Path -LiteralPath $PublicCard).Path) }
    Invoke-Probe $setupArguments
    $stock = Get-Content (Join-Path $output 'stock-setup.json') -Raw | ConvertFrom-Json
    $report.stock_public_key = $stock.after.key
    $dutStarted = $true
    if ($RawProbe) {
        $RawProbe = (Resolve-Path -LiteralPath $RawProbe).Path
        $report.raw_sha256 = (Get-FileHash $RawProbe -Algorithm SHA256).Hash.ToLowerInvariant()
        $rawArguments = @($config.port,$PeerPort,"$($config.frequency_hz)")
        if ($RawFloodText) {
            if ($RawFloodText -notmatch '^[a-zA-Z0-9-]{1,100}$') { throw 'Raw diagnostic text must be one explicit ASCII token.' }
            $rawArguments += @($(if ($RawDirectFirst) {'--direct-first'} else {'--flood-first'}),$RawFloodText)
        }
        $raw = Start-Process -FilePath (Resolve-Path -LiteralPath $RawProbe).Path -ArgumentList $rawArguments -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $output 'raw.stdout.log') -RedirectStandardError (Join-Path $output 'raw.stderr.log')
        try {
            if (!$raw.WaitForExit(180000)) { throw 'Raw hardware probe deadline exceeded.' }
            $report.raw_exit = $raw.ExitCode
            if ((Get-FileHash $RawProbe -Algorithm SHA256).Hash -ne $report.raw_sha256) { throw 'Raw executable changed during the run.' }
            if ($raw.ExitCode -ne 0) { throw "Raw hardware probe exited $($raw.ExitCode)." }
            $report.result = 'raw_probe_passed'
        } finally {
            if (!$raw.HasExited) { Stop-Process -Id $raw.Id; $raw.WaitForExit() }
        }
    } else {
        $nativeArgs = @{Protocol='meshcore';Port=$config.port;PeerPort=$PeerPort;SelfAddress=$SelfAddress;PeerAddress=$stock.after.key;Realm=$config.realm;OutputDirectory=(Join-Path $output 'native');Zoom=$Zoom;BenchPython=$Python;PeerUsbSerial=$PeerUsbSerial}
        if ($DjinnCustody) { $nativeArgs.TucketConfig = $BenchConfig } else { $nativeArgs.TucketBenchConfig = $BenchConfig }
        & (Join-Path $repo 'apps/signalman-desktop/scenarios/run_companion.ps1') @nativeArgs
        $report.result = 'native_completed_pending_visual_review'
    }
} catch {
    $failure = $_
    $report.error = $_.Exception.Message
    $report.result = 'failed'
} finally {
    Start-Sleep -Milliseconds 700
    if ($dutStarted) {
        try {
            Invoke-Probe @('restore-profile','--port',$config.port,'--usb-serial',$DutUsbSerial,'--baseline',(Join-Path $output 'dut-profile.json'),'--output',(Join-Path $output 'dut-restored-profile.json'))
            $report.restoration.dut_profile = 'acknowledged_same_boot_and_sync'
        } catch { $report.restoration.dut_error = $_.Exception.Message; if (!$failure) { $failure = $_ } }
    }
    if ($peerTouched) {
        try {
            # The stock image cannot provide Tulle status; re-identify the USB
            # hardware immediately before restoring its pinned original ELF.
            & $Python -B -c 'import sys; from serial.tools.list_ports import comports; p=next(p for p in comports() if p.device==sys.argv[1]); assert (p.vid,p.pid,p.serial_number)==(0x303a,0x1001,sys.argv[2])' $PeerPort $PeerUsbSerial
            if ($LASTEXITCODE -ne 0) { throw 'Peer USB identity changed before restore.' }
            & $helper flash --port $PeerPort --chip esp32s3 --before usb-reset --after watchdog-reset --baud 921600 --flash-size 16mb --non-interactive --skip-update-check $tulleImage *> (Join-Path $output 'tulle-restore.log')
            if ($LASTEXITCODE -ne 0) { throw 'Tulle restoration transfer failed.' }
            Start-Sleep -Seconds 2
            Invoke-Probe @('status','--port',$PeerPort,'--usb-serial',$PeerUsbSerial,'--output',(Join-Path $output 'peer-restored.json'))
            $original = Get-Content (Join-Path $output 'peer-before.json') -Raw | ConvertFrom-Json
            $restored = Get-Content (Join-Path $output 'peer-restored.json') -Raw | ConvertFrom-Json
            if ($original.status -ne $restored.status -or $original.region -ne $restored.region -or $original.channel -ne $restored.channel) { throw 'Peer Tulle status, region or channel changed.' }
            $report.restoration.peer = 'pinned_tulle_and_exact_status_region_channel'
        } catch { $report.restoration.peer_error = $_.Exception.Message; if (!$failure) { $failure = $_ } }
    }
    if ($failure) { $report.result = 'failed' }
    $report.closed_utc = [DateTime]::UtcNow.ToString('o')
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $output 'bench.json')
}
if ($failure) { throw $failure }
