# Actual Keeper/custody-route receipts. Never opens an installed user vault.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$HostBinary,
    [Parameter(Mandatory)][string]$ProbeBinary,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [switch]$ExpectFallbackFailure
)
$ErrorActionPreference='Stop'
$HostBinary=(Resolve-Path -LiteralPath $HostBinary).Path
$ProbeBinary=(Resolve-Path -LiteralPath $ProbeBinary).Path
$output=[IO.Path]::GetFullPath($OutputDirectory)
if((Test-Path -LiteralPath $output) -or $output -match '[\r\n"]'){throw 'Require a fresh explicit receipt directory.'}
New-Item -ItemType Directory -Path $output | Out-Null
$taskDevice=[guid]::NewGuid().ToString()
$realm='bench-tucket-custody'
$caseResults=@()
$brokerProcess=$null
$probeProcess=$null
$activeRoot=$null
$report=[ordered]@{schema=1;result='not_completed';device_id=$taskDevice;realm=$realm;source='4d744b8347dce72f695eab05815a4a472aed8355';host_sha256=(Get-FileHash $HostBinary).Hash.ToLowerInvariant();probe_sha256=(Get-FileHash $ProbeBinary).Hash.ToLowerInvariant();expected_fallback_failure=[bool]$ExpectFallbackFailure;cases=@();private_runtime_excluded=$true}
$report.source_sha256=(Get-Content -LiteralPath 'C:\Users\mark_\Code\repos\mere\testing\receipts\tucket-custody\restored-verified\qualification.json' -Raw|ConvertFrom-Json).source_sha256
$report.supplier_basis='previously qualified Tucket-only host and clean probe, byte-identical pinned binaries'
function Wait-Ready([string]$Path,[Diagnostics.Process]$Process){
    $until=[DateTime]::UtcNow.AddSeconds(20)
    while(!(Test-Path -LiteralPath $Path)){
        $Process.Refresh()
        if($Process.HasExited){throw 'Owned fixture exited before readiness.'}
        if([DateTime]::UtcNow -ge $until){throw 'Fixture readiness deadline.'}
        Start-Sleep -Milliseconds 25
    }
}
function Start-Broker([string]$Name,[string]$Mode){
    $script:activeRoot=Join-Path $output "runtime/$Name"
    $parent=Split-Path -Parent $script:activeRoot
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    [IO.File]::WriteAllText((Join-Path $parent '.gitignore'),"*`n")
    $endpoint='\\.\pipe\tucket-custody-'+[guid]::NewGuid().ToString('N')
    $script:brokerProcess=Start-Process -FilePath $HostBinary -ArgumentList @($endpoint,"`"$script:activeRoot`"",$Mode) -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $output "$Name-host.stdout.log") -RedirectStandardError (Join-Path $output "$Name-host.stderr.log")
    Wait-Ready (Join-Path $script:activeRoot 'ready') $script:brokerProcess
    return $endpoint
}
function Stop-Broker {
    if($script:brokerProcess){
        if(!$script:brokerProcess.HasExited){
            [IO.File]::WriteAllText((Join-Path $script:activeRoot 'stop'),'owned fixture shutdown')
            if(!$script:brokerProcess.WaitForExit(10000)){Stop-Process -Id $script:brokerProcess.Id;$script:brokerProcess.WaitForExit()}
        }
        $script:brokerProcess=$null
    }
}
function Invoke-Probe([string]$Name,[string]$Endpoint,[string]$Mode,[bool]$ExpectedFailure){
    $path=Join-Path $output "$Name.json"
    & $ProbeBinary $Endpoint $taskDevice $realm $path $Mode *> (Join-Path $output "$Name-probe.log")
    $exitCode=$LASTEXITCODE
    if($ExpectedFailure){
        $log=Get-Content -LiteralPath (Join-Path $output "$Name-probe.log") -Raw
        if($exitCode -eq 0 -or $log -notmatch 'forbidden fallback'){throw 'Restored fallback did not fail the pending control.'}
        return @{name=$Name;result='forbidden_fallback_detected';exit_code=$exitCode}
    }
    if($exitCode -ne 0){throw "Custody probe $Name failed: $exitCode"}
    $receipt=Get-Content -LiteralPath $path -Raw|ConvertFrom-Json
    return @{name=$Name;result=$receipt.result;exit_code=$exitCode;receipt_sha256=(Get-FileHash $path).Hash.ToLowerInvariant()}
}
function Watch-Custody([string]$Name,[string]$Endpoint,[bool]$ResidentLoss){
    $path=Join-Path $output "$Name.json"
    $script:probeProcess=Start-Process -FilePath $ProbeBinary -ArgumentList @($Endpoint,$taskDevice,$realm,"`"$path`"",'watch') -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $output "$Name-probe.stdout.log") -RedirectStandardError (Join-Path $output "$Name-probe.stderr.log")
    Wait-Ready ([IO.Path]::ChangeExtension($path,'ready')) $script:probeProcess
    if($ResidentLoss){Stop-Broker}else{[IO.File]::WriteAllText((Join-Path $script:activeRoot 'lock'),'owned fixture lock')}
    if(!$script:probeProcess.WaitForExit(15000)){throw 'Custody watch did not stop after lock/loss.'}
    if($script:probeProcess.ExitCode -ne 0){throw 'Custody watch probe failed.'}
    $receipt=Get-Content -LiteralPath $path -Raw|ConvertFrom-Json
    if($receipt.result -ne 'custody_closed'){throw 'Custody watch did not record closed state.'}
    $script:probeProcess=$null
    return @{name=$Name;result=$receipt.result;receipt_sha256=(Get-FileHash $path).Hash.ToLowerInvariant()}
}
try{
    $absent='\\.\pipe\tucket-custody-absent-'+[guid]::NewGuid().ToString('N')
    $caseResults+=Invoke-Probe 'absent' $absent 'pending' ([bool]$ExpectFallbackFailure)
    $endpoint=Start-Broker 'locked' 'locked'
    $caseResults+=Invoke-Probe 'locked' $endpoint 'pending' ([bool]$ExpectFallbackFailure)
    Stop-Broker
    if(!$ExpectFallbackFailure){
        $endpoint=Start-Broker 'no-wallet' 'no-wallet'
        $caseResults+=Invoke-Probe 'no-wallet' $endpoint 'pending' $false
        Stop-Broker
        $endpoint=Start-Broker 'unlocked' 'unlocked'
        $caseResults+=Invoke-Probe 'unlocked' $endpoint 'unlocked' $false
        $caseResults+=Watch-Custody 'lock-watch' $endpoint $false
        Stop-Broker
        $endpoint=Start-Broker 'resident-loss' 'unlocked'
        $caseResults+=Watch-Custody 'resident-loss' $endpoint $true
    }
    if((Get-FileHash $HostBinary).Hash -ne $report.host_sha256 -or (Get-FileHash $ProbeBinary).Hash -ne $report.probe_sha256){throw 'A custody executable changed during the run.'}
    $report.result=$(if($ExpectFallbackFailure){'restored_fallback_rejected'}else{'passed'})
} catch {$report.result='failed';$report.error=$_.Exception.Message;throw}
finally{
    if($probeProcess -and !$probeProcess.HasExited){Stop-Process -Id $probeProcess.Id;$probeProcess.WaitForExit()}
    Stop-Broker
    $report.generated_wallets_removed=$false
    $report.private_runtime='runtime/ (generated synthetic wallets; ignored by Git)'
    $report.cases=$caseResults
    $report.closed_utc=[DateTime]::UtcNow.ToString('o')
    $report|ConvertTo-Json -Depth 7|Set-Content -LiteralPath (Join-Path $output 'qualification.json')
}
