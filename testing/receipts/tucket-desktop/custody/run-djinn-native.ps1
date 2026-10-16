$ErrorActionPreference='Stop'
$repo='C:\Users\mark_\Code\repos\retinue'
$root=Join-Path $repo 'validation/results/tucket-desktop-20261015'
$hostBinary=(Resolve-Path 'C:\t\cargo-targets\mere\debug\tucket-custody-receipt-host.exe').Path
$cardBinary=(Resolve-Path (Join-Path $repo 'apps/signalman-desktop/target/debug/examples/tucket_contact_card.exe')).Path
$expected=Get-Content 'C:\Users\mark_\Code\repos\mere\testing\receipts\tucket-custody\restored-verified\qualification.json' -Raw|ConvertFrom-Json
if((Get-FileHash $hostBinary).Hash -ne $expected.host_sha256){throw 'Custodian fixture executable differs from its qualification.'}
$privateRoot=Join-Path $root 'djinn-radio-host'
if(Test-Path -LiteralPath $privateRoot){throw 'Preserve previous fixture wallet.'}
$card=Join-Path $root 'djinn-contact-card.json'
if(Test-Path -LiteralPath $card){throw 'Preserve previous contact card.'}
$endpoint='\\.\pipe\signalman-tucket-radio-'+[guid]::NewGuid().ToString('N')
$savedEndpoint=$env:GRAPHSHELL_APP_ENDPOINT
$savedStation=$env:SIGNALMAN_STATION_PORT
$custodian=$null
$report=[ordered]@{result='not_completed';authority='isolated Djinn wallet over real Keeper/custody route';host_sha256=$expected.host_sha256;radio_result='native-djinn/bench.json';private_wallet_tracked=$false}
try {
    $custodian=Start-Process -FilePath $hostBinary -ArgumentList @($endpoint,$privateRoot,'unlocked') -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $root 'djinn-host.stdout.log') -RedirectStandardError (Join-Path $root 'djinn-host.stderr.log')
    $until=[DateTime]::UtcNow.AddSeconds(20)
    while(!(Test-Path -LiteralPath (Join-Path $privateRoot 'ready'))){
        $custodian.Refresh()
        if($custodian.HasExited){throw 'Custodian fixture exited before readiness.'}
        if([DateTime]::UtcNow -ge $until){throw 'Custodian fixture startup deadline.'}
        Start-Sleep -Milliseconds 25
    }
    $env:GRAPHSHELL_APP_ENDPOINT=$endpoint
    [Environment]::SetEnvironmentVariable('SIGNALMAN_STATION_PORT',$null)
    & $cardBinary (Join-Path $root 'djinn-public-config.json') $card *> (Join-Path $root 'djinn-contact-card.log')
    if($LASTEXITCODE -ne 0){throw 'Djinn public contact-card export failed.'}
    $public=Get-Content $card -Raw|ConvertFrom-Json
    $report.public_key=$public.public_key
    $report.card_sha256=(Get-FileHash $card).Hash
    & (Join-Path $repo 'testing/run_tucket_desktop.ps1') -DjinnCustody -BenchConfig (Join-Path $root 'djinn-public-config.json') -PublicCard $card -SelfAddress $public.public_key -DutUsbSerial '44:1B:F6:6A:FA:64' -PeerPort COM6 -PeerUsbSerial '44:1B:F6:6A:FB:28' -OutputDirectory (Join-Path $root 'native-djinn') -Python (Join-Path $repo 'validation/results/sennet-tucket-radios/stock-cli/Scripts/python.exe') -DutBaseline (Join-Path $root 'baseline-com7-after-reconnect.json')
    $report.result='native_completed_pending_visual_review'
} catch {$report.result='failed';$report.error=$_.Exception.Message;throw}
finally {
    if($custodian -and !$custodian.HasExited){
        [IO.File]::WriteAllText((Join-Path $privateRoot 'stop'),'owned radio trial shutdown')
        if(!$custodian.WaitForExit(10000)){Stop-Process -Id $custodian.Id;$custodian.WaitForExit()}
    }
    [Environment]::SetEnvironmentVariable('GRAPHSHELL_APP_ENDPOINT',$savedEndpoint)
    [Environment]::SetEnvironmentVariable('SIGNALMAN_STATION_PORT',$savedStation)
    $report.custodian_stopped=$true
    $report.closed_utc=[DateTime]::UtcNow.ToString('o')
    $report|ConvertTo-Json -Depth 5|Set-Content (Join-Path $root 'djinn-radio-run.json')
}
