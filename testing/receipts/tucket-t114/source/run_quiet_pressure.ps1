$ErrorActionPreference='Stop'
$repo='C:\Users\mark_\Code\repos\retinue'
Set-Location $repo
$root=Join-Path $repo 'validation/results/tucket-t114'
$python=Join-Path $repo 'validation/results/sennet-tucket-radios/stock-cli/Scripts/python.exe'
$ops=Join-Path $repo 'testing/tucket_t114_ops.py'
$failure=$null
try {
    & $python -B $ops live (Join-Path $root 'quiet-pressure-install')
    if($LASTEXITCODE -ne 0){throw 'Pressure image install failed'}
    & $python -B $ops pressure (Join-Path $root 'quiet-pressure')
    if($LASTEXITCODE -ne 0){throw 'Memory reserve acceptance failed'}
} catch { $failure=$_ }
finally {
    & $python -B $ops restore (Join-Path $root 'quiet-pressure-restoration')
    if($LASTEXITCODE -ne 0){throw 'T114 original firmware restoration failed'}
}
if($failure){throw $failure}