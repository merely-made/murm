# Bounded Djinn-backed public-channel native bench with pinned restoration.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$SennetConfig,
    [Parameter(Mandatory)][string]$PublicStation,
    [Parameter(Mandatory)][string]$DutUsbSerial,
    [Parameter(Mandatory)][string]$PeerPort,
    [Parameter(Mandatory)][string]$PeerUsbSerial,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Python,
    [Parameter(Mandatory)][string]$DutBaseline
)
$ErrorActionPreference='Stop'
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$config=Get-Content -LiteralPath $SennetConfig -Raw|ConvertFrom-Json
$card=Get-Content -LiteralPath $PublicStation -Raw|ConvertFrom-Json
$output=[IO.Path]::GetFullPath($OutputDirectory)
if((Test-Path -LiteralPath $output) -or $config.realm -notlike 'bench-*' -or $config.public_channel -ne 'LongFast' -or $card.authority -ne 'djinn' -or $card.realm -ne $config.realm -or $card.device_id -ne $config.device_id -or $card.node_number -eq $config.peer_node -or $config.port -eq $PeerPort -or $DutUsbSerial -eq $PeerUsbSerial){throw 'Require a fresh, explicit public-channel bench with distinct identified boards and scoped Djinn station.'}
$Python=(Resolve-Path -LiteralPath $Python).Path
$SennetConfig=(Resolve-Path -LiteralPath $SennetConfig).Path
$DutBaseline=(Resolve-Path -LiteralPath $DutBaseline).Path
$artifacts=Join-Path $repo 'validation/results/sennet-tucket-radios/artifacts'
$helper=Join-Path $artifacts 'espflash.exe'
$stockImage=Join-Path $artifacts 'firmware-heltec-v4-2.7.26.54e0d8d.factory.bin'
$tulleImage=Join-Path $repo 'target/xtensa-esp32s3-none-elf/release/tulle-heltec-v4-phy'
$stockPin=Get-Content (Join-Path $repo 'validation/results/sennet-tucket-radios/attempts/meshtastic-flash.json') -Raw|ConvertFrom-Json
$tullePin=Get-Content (Join-Path $repo 'validation/results/stock-baseline-20261014/tulle-restore.json') -Raw|ConvertFrom-Json
if((Get-FileHash $helper).Hash -ne $stockPin.helper.binary_sha256 -or (Get-FileHash $helper).Hash -ne $tullePin.helper_sha256 -or (Get-FileHash $tulleImage).Hash -ne $tullePin.image_sha256){throw 'Pinned helper or Tulle image changed.'}
$image=[IO.File]::ReadAllBytes($stockImage)
$hasher=[Security.Cryptography.SHA256]::Create()
try {
    if($image.Length -ne 2182944 -or $stockPin.package_parts.Count -ne 3){throw 'Unexpected stock image shape'}
    foreach($part in $stockPin.package_parts){
        $hash=[Convert]::ToHexString($hasher.ComputeHash($image,[int]$part.offset,[int]$part.byte_length)).ToLowerInvariant()
        if($hash -ne $part.sha256){throw 'Factory image differs from pinned stock package parts'}
    }
} finally {$hasher.Dispose()}
New-Item -ItemType Directory -Path $output|Out-Null
$report=[ordered]@{schema=1;source=(git -C $repo rev-parse HEAD).Trim();result='not_completed';authority='djinn';public_channel='LongFast';dut_port=$config.port;peer_port=$PeerPort;dut_usb_serial=$DutUsbSerial;peer_usb_serial=$PeerUsbSerial;node_number=$card.node_number;peer_node=$config.peer_node;helper_sha256=$tullePin.helper_sha256;stock_image_sha256=(Get-FileHash $stockImage).Hash.ToLowerInvariant();tulle_image_sha256=$tullePin.image_sha256;source_sha256=@{};restoration=@{};opened_utc=[DateTime]::UtcNow.ToString('o')}
foreach($path in @('Cargo.toml','Cargo.lock','apps/signalman-desktop/Cargo.toml','apps/signalman-desktop/Cargo.lock','apps/signalman/src/radio_comms/sennet_ids.rs','apps/signalman/src/radio_comms/resident.rs','apps/signalman/src/radio_comms/carriage.rs','apps/signalman/examples/stock_companion_pair.rs','crates/radio-hand/src/instances.rs','crates/radio-hand/src/instances/switching.rs','crates/radio-hand/src/instances/traffic.rs','apps/signalman-desktop/src/resident.rs','apps/signalman-desktop/src/resident/sennet_station.rs','apps/signalman-desktop/src/state/messages.rs','apps/signalman-desktop/src/views/messages.rs','apps/signalman-desktop/src/companion_scenario.rs','apps/signalman-desktop/scenarios/run_companion.ps1','apps/signalman-desktop/scenarios/companion.scn.in','testing/setup_sennet_stock.py','testing/run_sennet_desktop.ps1')){$report.source_sha256[$path]=(Get-FileHash (Join-Path $repo $path)).Hash.ToLowerInvariant()}
$report.config_sha256=(Get-FileHash $SennetConfig).Hash.ToLowerInvariant()
$probe=Join-Path $repo 'testing/stock_tucket_bench.py'
function Invoke-Probe([string[]]$Arguments){ & $Python -B $probe @Arguments; if($LASTEXITCODE -ne 0){throw "Tulle probe failed: $($Arguments[0])"} }
$peerTouched=$false; $dutStarted=$false; $failure=$null
try {
    Invoke-Probe @('status','--port',$config.port,'--usb-serial',$DutUsbSerial,'--output',(Join-Path $output 'dut-before.json'))
    Invoke-Probe @('status','--port',$PeerPort,'--usb-serial',$PeerUsbSerial,'--output',(Join-Path $output 'peer-before.json'))
    Invoke-Probe @('snapshot','--port',$config.port,'--usb-serial',$DutUsbSerial,'--baseline',$DutBaseline,'--output',(Join-Path $output 'dut-profile.json'))
    Copy-Item -LiteralPath $config.packet_ids -Destination (Join-Path $output 'packet-ids-before.json')
    Copy-Item -LiteralPath ($config.packet_ids+'.initialized') -Destination (Join-Path $output 'packet-ids-fence.json')
    $peerTouched=$true
    & $helper write-bin --port $PeerPort --chip esp32s3 --before usb-reset --after watchdog-reset --baud 921600 --non-interactive --skip-update-check 0 $stockImage *> (Join-Path $output 'stock-flash.log')
    if($LASTEXITCODE -ne 0){throw 'Stock Meshtastic firmware transfer failed'}
    Start-Sleep -Seconds 4
    & $Python -B (Join-Path $PSScriptRoot 'setup_sennet_stock.py') --port $PeerPort --usb-serial $PeerUsbSerial --expected-node ('{0:x8}' -f [uint32]$config.peer_node) --configure --output (Join-Path $output 'stock-setup.json') *> (Join-Path $output 'stock-setup.log')
    if($LASTEXITCODE -ne 0){throw 'Stock public LongFast setup/readback failed'}
    $dutStarted=$true
    & (Join-Path $repo 'apps/signalman-desktop/scenarios/run_companion.ps1') -Protocol meshtastic -Port $config.port -PeerPort $PeerPort -SelfAddress $card.address -PeerAddress ('{0:x8}' -f [uint32]$config.peer_node) -Realm $config.realm -OutputDirectory (Join-Path $output 'native') -Zoom 0.75 -BenchPython $Python -PeerUsbSerial $PeerUsbSerial -SennetConfig $SennetConfig -ExchangeAfterReconnect
    $report.result='native_completed_pending_visual_review'
} catch {$failure=$_; $report.result='failed'; $report.error=$_.Exception.Message}
finally {
    Start-Sleep -Milliseconds 700
    if(Test-Path -LiteralPath $config.packet_ids){Copy-Item -LiteralPath $config.packet_ids -Destination (Join-Path $output 'packet-ids-after.json')}
    if($dutStarted){try {Invoke-Probe @('restore-profile','--port',$config.port,'--usb-serial',$DutUsbSerial,'--baseline',(Join-Path $output 'dut-profile.json'),'--output',(Join-Path $output 'dut-restored-profile.json')); $report.restoration.dut_profile='acknowledged_same_boot_and_sync'}catch{$report.restoration.dut_error=$_.Exception.Message; if(!$failure){$failure=$_}}}
    if($peerTouched){try {
        & $Python -B -c 'import sys; from serial.tools.list_ports import comports; p=next(p for p in comports() if p.device==sys.argv[1]); assert (p.vid,p.pid,p.serial_number)==(0x303a,0x1001,sys.argv[2])' $PeerPort $PeerUsbSerial
        if($LASTEXITCODE -ne 0){throw 'Peer USB identity changed before restoration'}
        & $helper flash --port $PeerPort --chip esp32s3 --before usb-reset --after watchdog-reset --baud 921600 --flash-size 16mb --non-interactive --skip-update-check $tulleImage *> (Join-Path $output 'tulle-restore.log')
        if($LASTEXITCODE -ne 0){throw 'Tulle restoration transfer failed'}
        Start-Sleep -Seconds 2
        Invoke-Probe @('status','--port',$PeerPort,'--usb-serial',$PeerUsbSerial,'--output',(Join-Path $output 'peer-restored.json'))
        $original=Get-Content (Join-Path $output 'peer-before.json') -Raw|ConvertFrom-Json
        $restored=Get-Content (Join-Path $output 'peer-restored.json') -Raw|ConvertFrom-Json
        if($original.status -ne $restored.status -or $original.region -ne $restored.region -or $original.channel -ne $restored.channel){throw 'Peer Tulle status/identity/region/channel changed'}
        $report.restoration.peer='pinned_tulle_and_exact_status_region_channel'
    }catch{$report.restoration.peer_error=$_.Exception.Message; if(!$failure){$failure=$_}}}
    if($failure){$report.result='failed'}
    $report.closed_utc=[DateTime]::UtcNow.ToString('o')
    $report|ConvertTo-Json -Depth 8|Set-Content -LiteralPath (Join-Path $output 'bench.json')
}
if($failure){throw $failure}
