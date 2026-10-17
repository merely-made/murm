# Attended native cached-contact trial. Application-only peer excursion.
$ErrorActionPreference='Stop'
$repo='C:\Users\mark_\Code\repos\retinue'
$root=Join-Path $repo 'validation/results/tucket-contacts'
$python=Join-Path $repo 'validation/results/sennet-tucket-radios/stock-cli/Scripts/python.exe'
$probe=Join-Path $repo 'testing/stock_tucket_bench.py'
$hostBinary='C:\t\cargo-targets\mere\debug\tucket-custody-receipt-host.exe'
$cardBinary=Join-Path $repo 'apps/signalman-desktop/target/debug/examples/tucket_contact_card.exe'
$privateRoot=Join-Path $root 'native-custodian'
$configPath=Join-Path $root 'public-config.json'
$cardPath=Join-Path $root 'public-card.json'
$reportPath=Join-Path $root 'native-trial.json'
if(Test-Path $reportPath){throw 'Preserve previous trial.'}
if(Test-Path $privateRoot){throw 'Preserve previous custodian.'}
if(!(Test-Path (Join-Path $root 'rollback.json'))){throw 'Require checksum-verified fresh rollback.'}
$supplier=Get-Content (Join-Path $repo 'testing/receipts/sennet-desktop/custody-supplier.json') -Raw|ConvertFrom-Json
if($supplier.result -ne 'passed' -or (Get-FileHash $hostBinary).Hash -ne $supplier.host_sha256){throw 'Qualified custodian changed.'}
$report=[ordered]@{passed=$false;source=(git -C $repo rev-parse HEAD);authority='isolated real Djinn Keeper wallet and scoped production custody route';source_sha256=@{};helper_sha256=(Get-FileHash $hostBinary).Hash;protected_writes=$false}
$paths=git -C $repo ls-files --cached --others --exclude-standard -- apps/signalman/src apps/signalman/examples/stock_companion_pair.rs apps/signalman-desktop/src apps/signalman-desktop/scenarios crates/tucket/src crates/radio-hand/src/instances.rs crates/radio-hand/src/instances
foreach($path in $paths){$report.source_sha256[$path]=(Get-FileHash (Join-Path $repo $path)).Hash.ToLowerInvariant()}
$config=Get-Content (Join-Path $repo 'validation/results/tucket-desktop-20261015/djinn-public-config.json') -Raw|ConvertFrom-Json
$config.realm='bench-tucket-cached-contacts'
$config.journal=Join-Path $root 'unused-config-journal.json'
$config|ConvertTo-Json -Depth 5|Set-Content $configPath
$savedEndpoint=$env:GRAPHSHELL_APP_ENDPOINT
$savedStation=$env:SIGNALMAN_STATION_PORT
$endpoint='\\.\pipe\signalman-tucket-radio-'+[guid]::NewGuid().ToString('N')
$custodian=$null
$touched=$false
$dutStarted=$false
$failure=$null
function Invoke-Python([string[]]$Arguments){
    & $python -B @Arguments
    if($LASTEXITCODE -ne 0){throw "Physical helper failed: $($Arguments[0]) $($Arguments[1])"}
}
try{
    Invoke-Python @($probe,'snapshot','--port','COM7','--usb-serial','44:1B:F6:6A:FA:64','--output',(Join-Path $root 'dut-profile.json'))
    $custodian=Start-Process -FilePath $hostBinary -ArgumentList @($endpoint,$privateRoot,'unlocked') -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $root 'custodian.stdout.log') -RedirectStandardError (Join-Path $root 'custodian.stderr.log')
    $until=[DateTime]::UtcNow.AddSeconds(20)
    while(!(Test-Path (Join-Path $privateRoot 'ready'))){
        $custodian.Refresh()
        if($custodian.HasExited -or [DateTime]::UtcNow -ge $until){throw 'Custodian startup failed.'}
        Start-Sleep -Milliseconds 50
    }
    $env:GRAPHSHELL_APP_ENDPOINT=$endpoint
    [Environment]::SetEnvironmentVariable('SIGNALMAN_STATION_PORT',$null)
    & $cardBinary $configPath $cardPath *> (Join-Path $root 'card-export.log')
    if($LASTEXITCODE -ne 0){throw 'Public card export failed.'}
    $card=Get-Content $cardPath -Raw|ConvertFrom-Json
    $report.self_public_key=$card.public_key
    $touched=$true
    Invoke-Python @((Join-Path $root 'stock_app_trial.py'),'install')
    Invoke-Python @($probe,'stock-setup','--port','COM6','--usb-serial','44:1B:F6:6A:FB:28','--peer-key',$card.public_key,'--public-card',$cardPath,'--frequency-khz','906875','--bandwidth-hz','250000','--sf','10','--cr','5','--power-dbm','0','--output',(Join-Path $root 'stock-setup.json'))
    $stock=Get-Content (Join-Path $root 'stock-setup.json') -Raw|ConvertFrom-Json
    $report.stock_public_key=$stock.after.key
    $dutStarted=$true
    & (Join-Path $repo 'apps/signalman-desktop/scenarios/run_companion.ps1') -Protocol meshcore -Port COM7 -PeerPort COM6 -SelfAddress $card.public_key -PeerAddress $stock.after.key -Realm $config.realm -OutputDirectory (Join-Path $root 'native') -TucketConfig $configPath -BenchPython $python -PeerUsbSerial '44:1B:F6:6A:FB:28' -ExchangeAfterReconnect -RequireCachedContacts -Zoom 0.75
    $report.native_completed=$true
}catch{
    $failure=$_
    $report.error=$_.Exception.Message
}finally{
    if($touched){
        try{Invoke-Python @((Join-Path $root 'stock_app_trial.py'),'restore');$report.peer_restored=$true}
        catch{$report.peer_restore_error=$_.Exception.Message;if(!$failure){$failure=$_}}
    }
    if($dutStarted){
        try{
            Invoke-Python @($probe,'restore-profile','--port','COM7','--usb-serial','44:1B:F6:6A:FA:64','--baseline',(Join-Path $root 'dut-profile.json'),'--output',(Join-Path $root 'dut-profile-restored.json'))
            Invoke-Python @((Join-Path $repo 'testing/device_names.py'),'SB2','--name','SB2','--output',(Join-Path $root 'sb2-restored.json'))
            $before=Get-Content (Join-Path $root 'sb2-before.json') -Raw|ConvertFrom-Json
            $after=Get-Content (Join-Path $root 'sb2-restored.json') -Raw|ConvertFrom-Json
            if(($before.sessions[0].responses|ConvertTo-Json -Compress) -ne ($after.sessions[0].responses|ConvertTo-Json -Compress)){throw 'SB2 named firmware state differs.'}
            $report.dut_restored=$true
        }catch{$report.dut_restore_error=$_.Exception.Message;if(!$failure){$failure=$_}}
    }
    if($custodian -and !$custodian.HasExited){
        [IO.File]::WriteAllText((Join-Path $privateRoot 'stop'),'owned trial shutdown')
        if(!$custodian.WaitForExit(10000)){Stop-Process -Id $custodian.Id;$custodian.WaitForExit()}
    }
    [Environment]::SetEnvironmentVariable('GRAPHSHELL_APP_ENDPOINT',$savedEndpoint)
    [Environment]::SetEnvironmentVariable('SIGNALMAN_STATION_PORT',$savedStation)
    $report.custodian_stopped=$true
    $report.passed=[bool]($report.native_completed -and $report.peer_restored -and $report.dut_restored -and !$failure)
    $report.closed_utc=[DateTime]::UtcNow.ToString('o')
    $report|ConvertTo-Json -Depth 8|Set-Content $reportPath
}
if($failure){throw $failure}
