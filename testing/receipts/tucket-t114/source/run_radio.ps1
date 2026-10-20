param([ValidatePattern('^[a-z][a-z0-9-]+$')][string]$Trial='radio',[int]$ExpectedFirst=0)
$ErrorActionPreference='Stop'
$repo='C:\Users\mark_\Code\repos\retinue'
Set-Location $repo
$root=Join-Path $repo 'validation/results/tucket-t114'
$out=Join-Path $root $Trial
if(Test-Path $out){throw 'Preserve earlier trial'}
New-Item -ItemType Directory $out | Out-Null
$python=Join-Path $repo 'validation/results/sennet-tucket-radios/stock-cli/Scripts/python.exe'
$ops=Join-Path $root 'v4_ops.py'
$t114=Join-Path $repo 'testing/tucket_t114_ops.py'
$probe=Join-Path $repo 'apps/signalman-desktop/target/debug/examples/tucket_firmware_contacts.exe'
$peerBinary=Join-Path $repo 'target/debug/examples/stock_companion_pair.exe'
$hostBinary='C:\Users\mark_\Code\repos\mere\target\build\debug\deps\tucket_custody_receipt_host.exe'
$cardBinary=Join-Path $repo 'apps/signalman-desktop/target/debug/examples/tucket_contact_card.exe'
$supplier=Get-Content (Join-Path $root 'custody-controls/qualification.json') -Raw | ConvertFrom-Json
if($supplier.result -ne 'passed' -or (Get-FileHash $hostBinary).Hash -ne $supplier.host_sha256){throw 'Custody supplier qualification changed'}
$session=Join-Path $root 'custody'
$sessionInfo=Join-Path $session 'session.json'
$saved=$env:GRAPHSHELL_APP_ENDPOINT
$failure=$null;$dut=$null;$peer=$null;$dutTouched=$false;$peerTouched=$false;$custodian=$null
$report=[ordered]@{passed=$false;firmware_source=(Get-Content (Join-Path $root 'quiet-images/source-revision.txt') -Raw).Trim();probe_source_sha256=(Get-FileHash (Join-Path $repo 'apps/signalman-desktop/examples/tucket_firmware_contacts.rs')).Hash;probe_sha256=(Get-FileHash $probe).Hash;peer_sha256=(Get-FileHash $peerBinary).Hash;custodian_sha256=$supplier.host_sha256;phases=@()}
function Invoke-Python([string[]]$Arguments){& $python -B @Arguments;if($LASTEXITCODE -ne 0){throw "Hardware helper failed: $($Arguments[0]) $($Arguments[1])"}}
try {
    if(!(Test-Path $sessionInfo)){
        New-Item -ItemType Directory $session | Out-Null
        $privateRoot=Join-Path $session 'custodian-private'
        $endpoint='\\.\pipe\signalman-tucket-t114-'+[guid]::NewGuid().ToString('N')
        $custodian=Start-Process -FilePath $hostBinary -ArgumentList @($endpoint,$privateRoot,'unlocked') -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $session 'custodian.stdout.log') -RedirectStandardError (Join-Path $session 'custodian.stderr.log')
        $until=[DateTime]::UtcNow.AddSeconds(20)
        while(!(Test-Path (Join-Path $privateRoot 'ready'))){$custodian.Refresh();if($custodian.HasExited -or [DateTime]::UtcNow -ge $until){throw 'Custodian readiness failed'};Start-Sleep -Milliseconds 50}
        @{pid=$custodian.Id;endpoint=$endpoint;private_root=$privateRoot}|ConvertTo-Json|Set-Content $sessionInfo
        $config=Get-Content (Join-Path $repo 'validation/results/tucket-desktop-20261015/djinn-public-config.json') -Raw|ConvertFrom-Json
        $config.port='COM10';$config.dtr=$true;$config.realm='bench-tucket-t114';$config.device_id=[guid]::NewGuid().ToString();$config.journal=Join-Path $session 'unused-journal.json'
        $config|ConvertTo-Json -Depth 5|Set-Content (Join-Path $session 'public-config.json')
    }
    $owned=Get-Content $sessionInfo -Raw|ConvertFrom-Json
    $custodian=Get-Process -Id $owned.pid
    if($custodian.Path -ne $hostBinary){throw 'Owned custodian identity mismatch'}
    $env:GRAPHSHELL_APP_ENDPOINT=$owned.endpoint
    $configPath=Join-Path $session 'public-config.json';$cardPath=Join-Path $session 'public-card.json'
    if(!(Test-Path $cardPath)){& $cardBinary $configPath $cardPath *> (Join-Path $session 'card-export.log');if($LASTEXITCODE -ne 0){throw 'Card export failed'}}
    $card=Get-Content $cardPath -Raw|ConvertFrom-Json
    $config=Get-Content $configPath -Raw|ConvertFrom-Json
    $report.self_public=$card.public_key
    $dutTouched=$true
    if(!(Test-Path (Join-Path $root 'fixture-handoff/receipt.json'))){Invoke-Python @($t114,'fixture-handoff',(Join-Path $root 'fixture-handoff'))}
    if(!(Get-Content (Join-Path $root 'fixture-handoff/receipt.json') -Raw|ConvertFrom-Json).passed){throw 'Fixture handoff did not pass'}
    Invoke-Python @($t114,'live',(Join-Path $out 'dut-install'))
    $memory=Get-Content (Join-Path $root 'quiet-pressure/board.json') -Raw|ConvertFrom-Json
    if(!$memory.passed -or $memory.requested_rounds -ne 16 -or $memory.acceptance.min_stack_bytes -ne 16384 -or $memory.acceptance.min_free_heap_percent -ne 25 -or !$memory.attached.Contains("build=$($report.firmware_source)")){throw 'Exact-image memory gate does not match'}
    $report.memory_gate='quiet-pressure/board.json'
    Invoke-Python @($ops,'baseline','SB1',"trial=$Trial")
    $peerTouched=$true
    Invoke-Python @($ops,'flash','SB1','stock-app.bin','stock-install',"trial=$Trial")
    Invoke-Python @((Join-Path $repo 'testing/stock_tucket_bench.py'),'stock-setup','--port','COM6','--usb-serial','44:1B:F6:6A:FB:28','--peer-key',$card.public_key,'--public-card',$cardPath,'--power-dbm','0','--output',(Join-Path $out 'stock-setup.json'))
    $stock=Get-Content (Join-Path $out 'stock-setup.json') -Raw|ConvertFrom-Json
    $report.peer_public=$stock.after.key
    $tag='signalman-ui-t114-'+[guid]::NewGuid().ToString('N')
    foreach($phase in 0,1){
        $name=if($phase -eq 0){'first'}else{'cold'}
        $phaseOut=Join-Path $out $name
        $phaseTag=if($phase -eq 0){$tag}else{"$tag-reconnect"}
        $expected=if($phase -eq 0){$ExpectedFirst}else{1}
        $dut=Start-Process -FilePath $probe -ArgumentList @($configPath,$phaseOut,$stock.after.key,$phaseTag,"$expected",'16384','25') -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $out "$name.stdout.log") -RedirectStandardError (Join-Path $out "$name.stderr.log")
        $until=[DateTime]::UtcNow.AddSeconds(30)
        while(!(Test-Path (Join-Path $phaseOut 'configured.json'))){$dut.Refresh();if($dut.HasExited -or [DateTime]::UtcNow -ge $until){throw "T114 $name configuration failed"};Start-Sleep -Milliseconds 100}
        $configured=Get-Content (Join-Path $phaseOut 'configured.json') -Raw|ConvertFrom-Json
        if($configured.public_key -ne $card.public_key){throw 'Custody identity changed'}
        if($phase -eq 0){Invoke-Python @((Join-Path $repo 'testing/stock_tucket_bench.py'),'advertise','--port','COM6','--usb-serial','44:1B:F6:6A:FB:28','--output',(Join-Path $out 'initial-stock-advertise.json'))}
        $peerOut=Join-Path $out "$name-peer"
        $peer=Start-Process -FilePath $peerBinary -ArgumentList @('peer','meshcore','COM6',$config.realm,$peerOut,$phaseTag,$card.public_key,'0') -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $out "$name-peer.stdout.log") -RedirectStandardError (Join-Path $out "$name-peer.stderr.log")
        [IO.File]::WriteAllText((Join-Path $phaseOut 'ready'),'identified peer ready')
        if(!$dut.WaitForExit(35000) -or $dut.ExitCode -ne 0){throw "T114 $name exchange failed"}
        if(!$peer.WaitForExit(15000) -or $peer.ExitCode -ne 0){throw "Stock $name exchange failed"}
        $result=Get-Content (Join-Path $phaseOut 'receipt.json') -Raw|ConvertFrom-Json
        $peerResult=Get-Content (Join-Path $peerOut 'receipt.json') -Raw|ConvertFrom-Json
        if(!$result.passed -or $peerResult.result -ne 'passed'){throw 'Exchange lacks passing receipts'}
        $report.phases+=@{name=$name;cached_before_peer_work=$configured.cached_before_peer_work;passed=$true}
        if($phase -eq 0){Invoke-Python @($t114,'reset',(Join-Path $out 'dut-cold-reset'))}
    }
    $report.radio_passed=$true
} catch { $failure=$_;$report.error=$_.Exception.Message }
finally {
    foreach($process in @($dut,$peer)){if($process -and !$process.HasExited){Stop-Process -Id $process.Id;$process.WaitForExit()}}
    if($peerTouched){try{Invoke-Python @($ops,'restore','SB1',"trial=$Trial");$report.peer_restored=$true}catch{$report.peer_restore_error=$_.Exception.Message;if(!$failure){$failure=$_}}}
    if($dutTouched){try{Invoke-Python @($t114,'restore',(Join-Path $out 'dut-restoration'));$report.dut_restored=$true}catch{$report.dut_restore_error=$_.Exception.Message;if(!$failure){$failure=$_}}}
    $report.passed=[bool]($report.radio_passed -and $report.peer_restored -and $report.dut_restored -and !$failure)
    if($custodian -and !$custodian.HasExited -and $report.passed){[IO.File]::WriteAllText((Join-Path $owned.private_root 'stop'),'owned successful campaign shutdown');if(!$custodian.WaitForExit(10000)){Stop-Process -Id $custodian.Id;$custodian.WaitForExit()}}
    $report.custodian_retained_for_owned_retry=[bool]($custodian -and !$custodian.HasExited)
    [Environment]::SetEnvironmentVariable('GRAPHSHELL_APP_ENDPOINT',$saved)
    $report|ConvertTo-Json -Depth 6|Set-Content (Join-Path $out 'radio.json')
}
if($failure){throw $failure}