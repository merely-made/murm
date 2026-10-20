$ErrorActionPreference='Stop'
$repo='C:\Users\mark_\Code\repos\retinue'
Set-Location $repo
$out=Join-Path $repo 'validation/results/tucket-t114/quiet-images'
New-Item -ItemType Directory -Path $out | Out-Null
$env:CARGO_HOME='C:\t\cargo-homes\mere-upstreams'
$env:CARGO_TARGET_DIR=Join-Path $repo 'target'
$env:CARGO_BUILD_BUILD_DIR=Join-Path $repo 'target/build'
$env:RUSTC='C:\Users\mark_\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\rustc.exe'
$env:RETINUE_FIRMWARE_REVISION=(git rev-parse HEAD)
$env:RETINUE_FIRMWARE_REVISION | Set-Content (Join-Path $out 'source-revision.txt')
$python=Join-Path $repo 'validation/results/sennet-tucket-radios/stock-cli/Scripts/python.exe'
foreach($profile in @(@('production','tucket-endpoint'),@('live','tucket-endpoint,memory-bench'))){
    $name=$profile[0];$features=$profile[1]
    cargo build -p tulle-t114-phy --release --features $features --target thumbv7em-none-eabihf --offline *> (Join-Path $out "$name-build.log")
    if($LASTEXITCODE -ne 0){throw "$name build failed"}
    & 'C:\Program Files\LLVM\bin\llvm-size.exe' -A target/thumbv7em-none-eabihf/release/tulle-t114-phy > (Join-Path $out "$name-sections.txt")
    Get-FileHash target/thumbv7em-none-eabihf/release/tulle-t114-phy | Select-Object Algorithm,Hash | ConvertTo-Json | Set-Content (Join-Path $out ($name+'-elf.json'))
    & 'C:\Program Files\LLVM\bin\llvm-objcopy.exe' -O binary target/thumbv7em-none-eabihf/release/tulle-t114-phy (Join-Path $out "t114-$name-raw.bin")
    if($LASTEXITCODE -ne 0){throw 'objcopy failed'}
    & $python -c "from pathlib import Path; p=Path(r'$out/t114-$name-raw.bin'); d=p.read_bytes(); d+=b'\xff'*(-len(d)%4); assert 0x26000+(len(d)+4095)//4096*4096<=0xe4000; p.with_name('t114-$name.bin').write_bytes(d)"
    if($LASTEXITCODE -ne 0){throw 'image range failed'}
    & C:\Users\mark_\AppData\Local\hermes\hermes-agent\venv\Scripts\adafruit-nrfutil.exe dfu genpkg --application (Join-Path $out "t114-$name.bin") --application-version 1 --dev-type 0x52 (Join-Path $out "t114-$name.zip") *> (Join-Path $out "$name-package.log")
    if($LASTEXITCODE -ne 0){throw 'package failed'}
}
& $python -c "from pathlib import Path; import hashlib,json; p=Path(r'$out'); data={f.name:dict(bytes=f.stat().st_size,sha256=hashlib.sha256(f.read_bytes()).hexdigest()) for f in p.iterdir() if f.suffix in ['.bin','.zip','.elf']}; (p/'images.json').write_text(json.dumps(data,indent=2)+'\n')"
if($LASTEXITCODE -ne 0){throw 'manifest failed'}
if((git rev-parse HEAD) -ne $env:RETINUE_FIRMWARE_REVISION){throw 'source revision changed'}