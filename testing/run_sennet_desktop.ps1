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
    [Parameter(Mandatory)][string]$DutBaseline,
    [switch]$SetupOnly,
    [switch]$PkiCapture,
    [switch]$PkiReplyCapture,
    [switch]$PkiAcknowledgeCapture,
    [switch]$DirectedNative,
    [ValidateRange(1000,60000)][int]$AckTimeoutMs=15000,
    [ValidateRange(1,4096)][int]$SeenCapacity=64,
    [ValidateRange(1,20)][int]$StockPowerDbm=1
)
$ErrorActionPreference='Stop'
if(([int][bool]$SetupOnly + [int][bool]$PkiCapture + [int][bool]$DirectedNative) -gt 1){throw 'Choose one setup, PKI capture, or directed native mode'}
if($PkiReplyCapture -and !$PkiCapture){throw 'Reply capture requires PKI capture mode'}
if($PkiAcknowledgeCapture -and !$PkiReplyCapture){throw 'ACK capture requires reply capture mode'}
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$config=Get-Content -LiteralPath $SennetConfig -Raw|ConvertFrom-Json
$card=Get-Content -LiteralPath $PublicStation -Raw|ConvertFrom-Json
if(($PkiCapture -or $DirectedNative) -and $card.pki_public_key -notmatch '^[0-9a-f]{64}$'){throw 'PKI mode requires a current exporter and its full public key before touching hardware'}
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
if($PkiCapture -or $DirectedNative){foreach($path in @('crates/sennet/src/pki.rs','crates/sennet/src/routing.rs','crates/sennet/src/instance.rs','crates/sennet/src/instance/directed.rs','crates/sennet/src/instance/state.rs','crates/sennet/src/instance/types.rs','crates/sennet/src/instance/error.rs','crates/sennet/Cargo.toml','apps/signalman/src/radio_comms.rs','apps/signalman-desktop/examples/sennet_pki_capture.rs','testing/sennet_pki_stock.py','testing/receipts/sennet-desktop/run-djinn-native.ps1')){$report.source_sha256[$path]=(Get-FileHash (Join-Path $repo $path)).Hash.ToLowerInvariant()}}
$probe=Join-Path $repo 'testing/stock_tucket_bench.py'
function Invoke-Probe([string[]]$Arguments){ & $Python -B $probe @Arguments; if($LASTEXITCODE -ne 0){throw "Tulle probe failed: $($Arguments[0])"} }
$peerTouched=$false; $dutStarted=$false; $failure=$null
$captureProcess=$null
$report.authority=$(if($SetupOnly){'not_activated_setup_only'}else{'djinn'})
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
    $peerBinary=Join-Path $repo 'target/debug/examples/stock_companion_pair.exe'
    $report.stock_probe_sha256=(Get-FileHash $peerBinary).Hash.ToLowerInvariant()
    & $peerBinary probe meshtastic $PeerPort $config.realm (Join-Path $output 'stock-probe') 0 *> (Join-Path $output 'stock-probe.log')
    $probePath=Join-Path $output 'stock-probe/receipt.json'
    $stockProbe=Get-Content $probePath -Raw|ConvertFrom-Json
    if($LASTEXITCODE -ne 0 -and $stockProbe.error -eq 'Meshtastic rebooted during handshake'){
        # Cold stock boot can interrupt the first read-only handshake. End that
        # owner, settle, then require a fresh nonce/identity handshake. No RF
        # command or production actor refusal is replayed/relaxed.
        Start-Sleep -Seconds 5
        & $peerBinary probe meshtastic $PeerPort $config.realm (Join-Path $output 'stock-probe-settled') 0 *> (Join-Path $output 'stock-probe-settled.log')
        $probePath=Join-Path $output 'stock-probe-settled/receipt.json'
        $stockProbe=Get-Content $probePath -Raw|ConvertFrom-Json
    }
    if($LASTEXITCODE -ne 0){throw 'Independent stock companion handshake failed before configuration'}
    if($stockProbe.result -ne 'passed' -or $stockProbe.left.identity -ne ('{0:x8}' -f [uint32]$config.peer_node)){throw 'Independent stock identity readback failed'}
    Start-Sleep -Seconds 2
    & $Python -B (Join-Path $PSScriptRoot 'setup_sennet_stock.py') --port $PeerPort --usb-serial $PeerUsbSerial --expected-node ('{0:x8}' -f [uint32]$config.peer_node) --power-dbm $StockPowerDbm --configure --output (Join-Path $output 'stock-setup.json') *> (Join-Path $output 'stock-setup.log')
    if($LASTEXITCODE -ne 0){throw 'Stock public LongFast setup/readback failed'}
    # Returning from the stock Python client to this serial owner can expose a
    # reboot indication. Settle that boundary with read-only handshakes before
    # launching the message scenario; never retry a possibly submitted send.
    Start-Sleep -Seconds 2
    & $peerBinary probe meshtastic $PeerPort $config.realm (Join-Path $output 'configured-stock-probe') 0 *> (Join-Path $output 'configured-stock-probe.log')
    $probePath=Join-Path $output 'configured-stock-probe/receipt.json'
    $stockProbe=Get-Content $probePath -Raw|ConvertFrom-Json
    if($LASTEXITCODE -ne 0 -and $stockProbe.error -eq 'Meshtastic rebooted during handshake'){
        Start-Sleep -Seconds 5
        & $peerBinary probe meshtastic $PeerPort $config.realm (Join-Path $output 'configured-stock-probe-settled') 0 *> (Join-Path $output 'configured-stock-probe-settled.log')
        $probePath=Join-Path $output 'configured-stock-probe-settled/receipt.json'
        $stockProbe=Get-Content $probePath -Raw|ConvertFrom-Json
    }
    if($LASTEXITCODE -ne 0 -or $stockProbe.result -ne 'passed' -or $stockProbe.left.identity -ne ('{0:x8}' -f [uint32]$config.peer_node)){throw 'Configured stock peer did not pass its independent handshake'}
    if($PkiCapture -or $DirectedNative){
        $oracle=Join-Path $PSScriptRoot 'sennet_pki_stock.py'
        $stockSetup=Join-Path $output 'stock-setup.json'
        $stock=Get-Content $stockSetup -Raw|ConvertFrom-Json
        & $Python -B $oracle contact --port $PeerPort --usb-serial $PeerUsbSerial --station $PublicStation --setup $stockSetup --output (Join-Path $output 'public-contact-import.json') *> (Join-Path $output 'contact-import.log')
        if($LASTEXITCODE -ne 0){throw 'Stock public contact import/readback failed'}
        # Contact setup leaves this port unattended for a while. Require a
        # fresh same-boot status/profile readback immediately before ownership
        # passes to the Rust capture. Any subsequent startup rejection still
        # fails the entire trial; no RF command is replayed.
        Invoke-Probe @('snapshot','--port',$config.port,'--usb-serial',$DutUsbSerial,'--baseline',$DutBaseline,'--output',(Join-Path $output 'dut-before-capture.json'))
        if($DirectedNative){
            if($stock.pki_public_key -notmatch '^[0-9a-f]{64}$'){throw 'Stock readback omitted its public PKI key'}
            if($config.directed){
                if($config.directed.peer_public_key -ne $stock.pki_public_key){throw 'Pinned stock peer key changed; select the original identity or a new realm/journal'}
            }else{
                $config | Add-Member -NotePropertyName directed -NotePropertyValue ([ordered]@{peer_public_key=$stock.pki_public_key;ack_timeout_ms=$AckTimeoutMs;seen_capacity=$SeenCapacity})
            }
            $SennetConfig=Join-Path $output 'directed-activation-config.json'
            $config | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $SennetConfig
            $report.activation_config_sha256=(Get-FileHash $SennetConfig).Hash.ToLowerInvariant()
            # The contact importer was the last Python serial owner. Settle
            # this exact boundary before either actor can submit a message.
            Start-Sleep -Seconds 2
            & $peerBinary probe meshtastic $PeerPort $config.realm (Join-Path $output 'contact-stock-probe') 0 *> (Join-Path $output 'contact-stock-probe.log')
            $probePath=Join-Path $output 'contact-stock-probe/receipt.json'
            $stockProbe=Get-Content $probePath -Raw|ConvertFrom-Json
            if($LASTEXITCODE -ne 0 -and $stockProbe.error -eq 'Meshtastic rebooted during handshake'){
                Start-Sleep -Seconds 5
                & $peerBinary probe meshtastic $PeerPort $config.realm (Join-Path $output 'contact-stock-probe-settled') 0 *> (Join-Path $output 'contact-stock-probe-settled.log')
                $probePath=Join-Path $output 'contact-stock-probe-settled/receipt.json'
                $stockProbe=Get-Content $probePath -Raw|ConvertFrom-Json
            }
            if($LASTEXITCODE -ne 0 -or $stockProbe.result -ne 'passed' -or $stockProbe.left.identity -ne ('{0:x8}' -f [uint32]$config.peer_node)){throw 'Contact-configured stock peer did not pass its independent handshake'}
        }
    }
    if($PkiCapture){
        $capture=Join-Path $repo 'apps/signalman-desktop/target/debug/examples/sennet_pki_capture.exe'
        $report.capture_sha256=(Get-FileHash $capture).Hash.ToLowerInvariant()
        $captureOutput=Join-Path $output 'stock-pki-capture.json'
        $ready=[IO.Path]::ChangeExtension($captureOutput,'ready')
        $tag='sennet-pki-'+[guid]::NewGuid().ToString('N')
        $dutStarted=$true
        $captureArgs=@("`"$SennetConfig`"",$stock.pki_public_key,$tag,"`"$captureOutput`"")
        if($PkiAcknowledgeCapture){$captureArgs+='ack-reply'}elseif($PkiReplyCapture){$captureArgs+='reply'}
        $captureProcess=Start-Process -FilePath $capture -ArgumentList $captureArgs -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $output 'capture.stdout.log') -RedirectStandardError (Join-Path $output 'capture.stderr.log')
        $until=[DateTime]::UtcNow.AddSeconds(20)
        while(!(Test-Path $ready)){$captureProcess.Refresh();if($captureProcess.HasExited){throw 'PKI receive probe exited before readiness'};if([DateTime]::UtcNow -ge $until){throw 'PKI capture readiness deadline'};Start-Sleep -Milliseconds 100}
        $ackArgs=@();if($PkiAcknowledgeCapture){$ackArgs+='--require-ack'}
        & $Python -B $oracle send --port $PeerPort --usb-serial $PeerUsbSerial --station $PublicStation --setup $stockSetup --text $tag --output (Join-Path $output 'stock-pki-send.json') @ackArgs *> (Join-Path $output 'stock-pki-send.log')
        if($LASTEXITCODE -ne 0){throw 'Stock tagged PKI send command failed'}
        $until=[DateTime]::UtcNow.AddSeconds(95)
        while(!$captureProcess.WaitForExit(1000)){if([DateTime]::UtcNow -ge $until){throw 'PKI receive probe absolute deadline'}}
        if($captureProcess.ExitCode -ne 0){throw 'PKI receive probe did not authenticate and match the stock text'}
        if($PkiAcknowledgeCapture){
            $observed=Get-Content $captureOutput -Raw|ConvertFrom-Json
            $sent=Get-Content (Join-Path $output 'stock-pki-send.json') -Raw|ConvertFrom-Json
            if($observed.result -ne 'matched_stock_routing_success_not_native_gate' -or !$observed.pki_reply_transmitted -or !$observed.ack_frame_hex -or !$sent.matched_client_protocol_ack){throw 'ACK wire probe evidence incomplete'}
        }
        $report.result=$(if($PkiAcknowledgeCapture){'stock_protocol_ack_exchange_observed_not_native_gate'}elseif($PkiReplyCapture){'stock_routing_candidate_observed_no_ack_or_native_gate'}else{'stock_pki_receive_observed_no_ack_or_native_gate'})
    }elseif($SetupOnly){$report.result='stock_setup_passed_native_not_run'}else{
        $dutStarted=$true
        & (Join-Path $repo 'apps/signalman-desktop/scenarios/run_companion.ps1') -Protocol meshtastic -Port $config.port -PeerPort $PeerPort -SelfAddress $card.address -PeerAddress ('{0:x8}' -f [uint32]$config.peer_node) -Realm $config.realm -OutputDirectory (Join-Path $output 'native') -Zoom 0.75 -BenchPython $Python -PeerUsbSerial $PeerUsbSerial -SennetConfig $SennetConfig -SennetDirected:$DirectedNative -ExchangeAfterReconnect
        $report.result='native_completed_pending_visual_review'
    }
} catch {$failure=$_; $report.result='failed'; $report.error=$_.Exception.Message}
finally {
    if($captureProcess -and !$captureProcess.HasExited){Stop-Process -Id $captureProcess.Id;$captureProcess.WaitForExit()}
    Start-Sleep -Milliseconds 700
    if(Test-Path -LiteralPath $config.packet_ids){Copy-Item -LiteralPath $config.packet_ids -Destination (Join-Path $output 'packet-ids-after.json')}
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
    # End stock RF before the DUT's exact textual status/sync comparison.
    # Pending stock packets can otherwise interleave binary RX with those
    # diagnostics. Keep the same strict check and preserve any failure.
    if($dutStarted){try {Invoke-Probe @('restore-profile','--port',$config.port,'--usb-serial',$DutUsbSerial,'--baseline',(Join-Path $output 'dut-profile.json'),'--output',(Join-Path $output 'dut-restored-profile.json')); $report.restoration.dut_profile='acknowledged_same_boot_and_sync'}catch{$report.restoration.dut_error=$_.Exception.Message; if(!$failure){$failure=$_}}}
    if($failure){$report.result='failed'}
    $report.closed_utc=[DateTime]::UtcNow.ToString('o')
    $report|ConvertTo-Json -Depth 8|Set-Content -LiteralPath (Join-Path $output 'bench.json')
}
if($failure){throw $failure}
