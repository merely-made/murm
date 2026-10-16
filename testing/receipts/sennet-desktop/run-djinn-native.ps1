# An isolated real Keeper, explicit public LongFast configuration and nonce
# commissioning, followed by the guarded two-connection native/restoration gate.
[CmdletBinding()]
param([ValidatePattern('^[a-z][a-z0-9-]{2,48}$')][string]$TrialName='native-djinn-channel',[switch]$PkiCapture,[switch]$PkiReplyCapture,[switch]$PkiAcknowledgeCapture)
$ErrorActionPreference='Stop'
$repo=(Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$root=Join-Path $repo 'validation/results/sennet-desktop'
$hostBinary=(Resolve-Path 'C:\t\cargo-targets\mere\debug\tucket-custody-receipt-host.exe').Path
$stationBinary=(Resolve-Path (Join-Path $repo 'apps/signalman-desktop/target/debug/examples/sennet_station.exe')).Path
$expected=Get-Content (Join-Path $PSScriptRoot 'custody-supplier.json') -Raw|ConvertFrom-Json
if((Get-FileHash $hostBinary).Hash -ne $expected.host_sha256){throw 'Custodian fixture differs from its qualified producer.'}
$trial=Join-Path $root $TrialName
$privateRoot=Join-Path $root "$TrialName-custodian"
$public=Join-Path $root "$TrialName-station.json"
$configPath=Join-Path $root "$TrialName-public-config.json"
$ids=Join-Path $root "$TrialName-packet-ids.json"
foreach($path in @($trial,$privateRoot,$public,$configPath,$ids,($ids+'.initialized'))){if(Test-Path -LiteralPath $path){throw 'Preserve previous evidence and commissioned nonce state.'}}
$config=[ordered]@{device_id='cbdb40d6-d433-4679-a355-86aea5aabb61';port='COM7';realm='bench-sennet-djinn';journal=(Join-Path $trial 'native/desktop-journal.json');packet_ids=$ids;name='Signalman Sennet bench';public_channel='LongFast';peer_node=[Convert]::ToUInt32('f66afb28',16);frequency_hz=906875000;tx_power_dbm=0;tx_budget_ms=2000;frame_ttl_ms=15000;reservation_size=256;airtime_window_ms=60000;airtime_allowance_ms=12000;receive_wait_ms=50;open_settle_ms=800;online_timeout_ms=3000;transmit_timeout_ms=3000;dtr=$false}
[IO.File]::WriteAllText($configPath,($config|ConvertTo-Json -Depth 4))
$endpoint='\\.\pipe\signalman-sennet-radio-'+[guid]::NewGuid().ToString('N')
$oldEndpoint=$env:GRAPHSHELL_APP_ENDPOINT
$oldStation=$env:SIGNALMAN_STATION_PORT
$custodian=$null
$report=[ordered]@{result='not_completed';authority='isolated Djinn wallet through real Keeper route';host_sha256=$expected.host_sha256;station_sha256=(Get-FileHash $stationBinary).Hash.ToLowerInvariant();private_wallet_tracked=$false;radio_result="$TrialName/bench.json"}
try {
    $custodian=Start-Process -FilePath $hostBinary -ArgumentList @($endpoint,$privateRoot,'unlocked') -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $root "$TrialName-host.stdout.log") -RedirectStandardError (Join-Path $root "$TrialName-host.stderr.log")
    $until=[DateTime]::UtcNow.AddSeconds(20)
    while(!(Test-Path -LiteralPath (Join-Path $privateRoot 'ready'))){$custodian.Refresh();if($custodian.HasExited){throw 'Custodian exited before readiness'};if([DateTime]::UtcNow -ge $until){throw 'Custodian startup deadline'};Start-Sleep -Milliseconds 25}
    $env:GRAPHSHELL_APP_ENDPOINT=$endpoint
    [Environment]::SetEnvironmentVariable('SIGNALMAN_STATION_PORT',$null)
    & $stationBinary $configPath $public initialize-packet-ids *> (Join-Path $root "$TrialName-station.log")
    if($LASTEXITCODE -ne 0){throw 'Djinn station/nonce commissioning refused'}
    $card=Get-Content $public -Raw|ConvertFrom-Json
    $report.node_number=$card.node_number
    & (Join-Path $repo 'testing/run_sennet_desktop.ps1') -SennetConfig $configPath -PublicStation $public -DutUsbSerial '44:1B:F6:6A:FA:64' -PeerPort COM6 -PeerUsbSerial '44:1B:F6:6A:FB:28' -OutputDirectory $trial -Python (Join-Path $repo 'validation/results/sennet-tucket-radios/stock-cli/Scripts/python.exe') -DutBaseline (Join-Path $repo 'validation/results/tucket-desktop-20261015/baseline-com7-after-reconnect.json') -PkiCapture:$PkiCapture -PkiReplyCapture:$PkiReplyCapture -PkiAcknowledgeCapture:$PkiAcknowledgeCapture
    if((Get-FileHash $hostBinary).Hash -ne $expected.host_sha256){throw 'Custodian binary changed during native trial'}
    $report.result=$(if($PkiAcknowledgeCapture){'stock_protocol_ack_exchange_observed_not_native_gate'}elseif($PkiReplyCapture){'stock_routing_candidate_observed_no_ack_or_native_gate'}elseif($PkiCapture){'stock_pki_receive_observed_no_ack_or_native_gate'}else{'native_completed_pending_visual_review'})
} catch {$report.result='failed';$report.error=$_.Exception.Message;throw}
finally {
    if($custodian -and !$custodian.HasExited){[IO.File]::WriteAllText((Join-Path $privateRoot 'stop'),'owned Sennet trial shutdown');if(!$custodian.WaitForExit(10000)){Stop-Process -Id $custodian.Id;$custodian.WaitForExit()}}
    [Environment]::SetEnvironmentVariable('GRAPHSHELL_APP_ENDPOINT',$oldEndpoint)
    [Environment]::SetEnvironmentVariable('SIGNALMAN_STATION_PORT',$oldStation)
    $report.custodian_stopped=$true
    $report.closed_utc=[DateTime]::UtcNow.ToString('o')
    $report|ConvertTo-Json -Depth 6|Set-Content (Join-Path $root "$TrialName-custody.json")
}
