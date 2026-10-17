param([ValidatePattern('^[a-z][a-z0-9-]+$')][string]$Trial='rf-cold')
$ErrorActionPreference='Stop'
$repo='C:\Users\mark_\Code\repos\retinue'
$base=Join-Path $repo 'validation/results/tucket-firmware-contacts'
$root=Join-Path $base 'corrected'
$out=Join-Path $root $Trial
if(Test-Path $out){throw 'Preserve earlier radio trial.'}
if(!(Get-Content (Join-Path $root 'storage-cold-retry/v4-storage.json') -Raw|ConvertFrom-Json).passed){throw 'Require completed storage/restoration qualification.'}
$python=Join-Path $repo 'validation/results/sennet-tucket-radios/stock-cli/Scripts/python.exe'
$ops=Join-Path $base 'live_ops.py'
$probe=Join-Path $repo 'apps/signalman-desktop/target/debug/examples/tucket_firmware_contacts.exe'
$peerBinary=Join-Path $repo 'target/debug/examples/stock_companion_pair.exe'
$hostBinary='C:\t\cargo-targets\mere\debug\tucket-custody-receipt-host.exe'
$cardBinary=Join-Path $repo 'apps/signalman-desktop/target/debug/examples/tucket_contact_card.exe'
$supplier=Get-Content (Join-Path $repo 'testing/receipts/sennet-desktop/custody-supplier.json') -Raw|ConvertFrom-Json
if((Get-FileHash $hostBinary).Hash -ne $supplier.host_sha256){throw 'Qualified Djinn fixture changed.'}
New-Item -ItemType Directory -Path $out|Out-Null
$privateRoot=Join-Path $out 'custodian-private'
$configPath=Join-Path $out 'public-config.json'
$cardPath=Join-Path $out 'public-card.json'
$config=Get-Content (Join-Path $repo 'validation/results/tucket-desktop-20261015/djinn-public-config.json') -Raw|ConvertFrom-Json
$config.realm='bench-tucket-firmware-contacts';$config.journal=Join-Path $out 'unused-journal.json'
$config|ConvertTo-Json -Depth 5|Set-Content $configPath
$endpoint='\\.\pipe\signalman-tucket-radio-'+[guid]::NewGuid().ToString('N')
$saved=$env:GRAPHSHELL_APP_ENDPOINT
$report=[ordered]@{passed=$false;source=(Get-Content (Join-Path $root 'source-revision.txt') -Raw);authority='isolated real Keeper wallet, volatile USB firmware setup';custodian_sha256=$supplier.host_sha256;probe_sha256=(Get-FileHash $probe).Hash;peer_sha256=(Get-FileHash $peerBinary).Hash;phases=@()}
$custodian=$null;$dut=$null;$peer=$null;$dutTouched=$false;$peerTouched=$false;$failure=$null
function Invoke-Python([string[]]$Arguments){& $python -B @Arguments;if($LASTEXITCODE -ne 0){throw "Radio helper failed: $($Arguments[0]) $($Arguments[1])"}}
try{
    $custodian=Start-Process -FilePath $hostBinary -ArgumentList @($endpoint,$privateRoot,'unlocked') -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $out 'custodian.stdout.log') -RedirectStandardError (Join-Path $out 'custodian.stderr.log')
    $until=[DateTime]::UtcNow.AddSeconds(20)
    while(!(Test-Path (Join-Path $privateRoot 'ready'))){if($custodian.HasExited -or [DateTime]::UtcNow -ge $until){throw 'Custodian readiness failed'};Start-Sleep -Milliseconds 50;$custodian.Refresh()}
    $env:GRAPHSHELL_APP_ENDPOINT=$endpoint
    & $cardBinary $configPath $cardPath *> (Join-Path $out 'card-export.log')
    if($LASTEXITCODE -ne 0){throw 'Public card export failed.'}
    $card=Get-Content $cardPath -Raw|ConvertFrom-Json
    $report.self_public=$card.public_key
    $dutTouched=$true
    Invoke-Python @($ops,'flash','SB2','v4-live-app.bin','dut-live-install',"trial=$Trial")
    $stockImage=Join-Path $repo 'validation/results/sennet-tucket-radios/artifacts/heltec_v4_companion_radio_usb-v1.17.1-d929643-merged.bin'
    if((Get-FileHash $stockImage).Hash -ne '67ef59c7687787255c23659e7525b81c1fc88bf679521f4183099a50580d0a3d'){throw 'Stock pin changed.'}
    if(!(Test-Path (Join-Path $root 'stock-app.bin'))){[IO.File]::WriteAllBytes((Join-Path $root 'stock-app.bin'),([IO.File]::ReadAllBytes($stockImage)[65536..707263]))}
    $peerTouched=$true
    Invoke-Python @($ops,'flash','SB1','stock-app.bin','stock-live-install',"trial=$Trial")
    Invoke-Python @((Join-Path $repo 'testing/stock_tucket_bench.py'),'stock-setup','--port','COM6','--usb-serial','44:1B:F6:6A:FB:28','--peer-key',$card.public_key,'--public-card',$cardPath,'--power-dbm','0','--output',(Join-Path $out 'stock-setup.json'))
    $stock=Get-Content (Join-Path $out 'stock-setup.json') -Raw|ConvertFrom-Json
    $report.peer_public=$stock.after.key
    $tag='signalman-ui-fwcontacts-'+[guid]::NewGuid().ToString('N')
    foreach($phase in 0,1){
        $phaseName=if($phase -eq 0){'first'}else{'cold'}
        $phaseOut=Join-Path $out $phaseName
        $phaseTag=if($phase -eq 0){$tag}else{"$tag-reconnect"}
        $dut=Start-Process -FilePath $probe -ArgumentList @($configPath,$phaseOut,$stock.after.key,$phaseTag,"$phase",'16384','25') -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $out "$phaseName.stdout.log") -RedirectStandardError (Join-Path $out "$phaseName.stderr.log")
        $until=[DateTime]::UtcNow.AddSeconds(30)
        while(!(Test-Path (Join-Path $phaseOut 'configured.json'))){$dut.Refresh();if($dut.HasExited -or [DateTime]::UtcNow -ge $until){throw "Firmware $phaseName configuration failed"};Start-Sleep -Milliseconds 100}
        Start-Sleep -Milliseconds 150
        $configured=Get-Content (Join-Path $phaseOut 'configured.json') -Raw|ConvertFrom-Json
        if($configured.public_key -ne $card.public_key){throw 'Firmware custody identity changed.'}
        if($phase -eq 0){Invoke-Python @((Join-Path $repo 'testing/stock_tucket_bench.py'),'advertise','--port','COM6','--usb-serial','44:1B:F6:6A:FB:28','--output',(Join-Path $out 'initial-stock-advertise.json'))}
        $peerOut=Join-Path $out "$phaseName-peer"
        $peer=Start-Process -FilePath $peerBinary -ArgumentList @('peer','meshcore','COM6',$config.realm,$peerOut,$phaseTag,$card.public_key,'0') -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $out "$phaseName-peer.stdout.log") -RedirectStandardError (Join-Path $out "$phaseName-peer.stderr.log")
        [IO.File]::WriteAllText((Join-Path $phaseOut 'ready'),'identified peer ready')
        if(!$dut.WaitForExit(35000)){throw 'Firmware exchange deadline.'}
        if($dut.ExitCode -ne 0){throw "Firmware $phaseName exchange failed."}
        if(!$peer.WaitForExit(15000) -or $peer.ExitCode -ne 0){throw 'Stock peer exchange failed.'}
        $result=Get-Content (Join-Path $phaseOut 'receipt.json') -Raw|ConvertFrom-Json
        $peerResult=Get-Content (Join-Path $peerOut 'receipt.json') -Raw|ConvertFrom-Json
        if(!$result.passed -or $peerResult.result -ne 'passed'){throw 'An exchange omitted its passing receipt.'}
        $report.phases+=@{name=$phaseName;cached_before_peer_work=$configured.cached_before_peer_work;passed=$true}
        if($phase -eq 0){Invoke-Python @($ops,'reset','SB2','dut-cold-reset',"trial=$Trial")}
    }
    $report.radio_passed=$true
}catch{$failure=$_;$report.error=$_.Exception.Message}
finally{
    foreach($process in @($dut,$peer)){if($process -and !$process.HasExited){Stop-Process -Id $process.Id;$process.WaitForExit()}}
    if($peerTouched){try{Invoke-Python @($ops,'restore','SB1',"trial=$Trial");$report.peer_restored=$true}catch{$report.peer_restore_error=$_.Exception.Message;if(!$failure){$failure=$_}}}
    if($dutTouched){try{Invoke-Python @($ops,'restore','SB2',"trial=$Trial");$report.dut_restored=$true}catch{$report.dut_restore_error=$_.Exception.Message;if(!$failure){$failure=$_}}}
    if($custodian -and !$custodian.HasExited){[IO.File]::WriteAllText((Join-Path $privateRoot 'stop'),'owned trial shutdown');if(!$custodian.WaitForExit(10000)){Stop-Process -Id $custodian.Id;$custodian.WaitForExit()}}
    [Environment]::SetEnvironmentVariable('GRAPHSHELL_APP_ENDPOINT',$saved)
    $report.custodian_stopped=$true;$report.passed=[bool]($report.radio_passed -and $report.peer_restored -and $report.dut_restored -and !$failure)
    $report.closed_utc=[DateTime]::UtcNow.ToString('o')
    $report|ConvertTo-Json -Depth 5|Set-Content (Join-Path $out 'radio.json')
}
if($failure){throw $failure}
