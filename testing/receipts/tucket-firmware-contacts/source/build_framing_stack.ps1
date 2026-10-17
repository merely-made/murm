$ErrorActionPreference='Stop'
$repo='C:\Users\mark_\Code\repos\retinue'
$out=Join-Path $repo 'validation/results/tucket-firmware-contacts/framing-stack'
if(git -C $repo status --porcelain){throw 'Require clean source before pinned builds.'}
$env:CARGO_HOME='C:\t\cargo-homes\mere-upstreams'
$env:CARGO_TARGET_DIR=Join-Path $repo 'target'
$env:CARGO_BUILD_BUILD_DIR=Join-Path $repo 'target/build'
$env:CARGO_NET_OFFLINE='true'
$env:RETINUE_FIRMWARE_REVISION=(git -C $repo rev-parse HEAD)
[IO.File]::WriteAllText((Join-Path $out 'source-revision.txt'),$env:RETINUE_FIRMWARE_REVISION)
$esp=Join-Path $repo 'validation/results/sennet-tucket-radios/artifacts/espflash.exe'
$xtensaObjdump='C:\Users\mark_\.rustup\toolchains\esp\xtensa-esp-elf\bin\xtensa-esp32s3-elf-objdump.exe'
$armElf=Join-Path $repo 'target/thumbv7em-none-eabihf/release/tulle-t114-phy'
$espElf=Join-Path $repo 'target/xtensa-esp32s3-none-elf/release/tulle-heltec-v4-phy'
foreach($variant in @([pscustomobject]@{Name='live';Features='resident-contact-store,memory-bench'})){
    $name=$variant.Name
    & cargo +esp build -p tulle-heltec-v4-phy --release --features $variant.Features --target xtensa-esp32s3-none-elf --ignore-rust-version '-Zbuild-std=core,alloc' --offline *> (Join-Path $out "v4-$name-build.log")
    if($LASTEXITCODE -ne 0){throw "V4 $name build failed"}
    & $esp save-image --chip esp32s3 --flash-size 16mb --merge --skip-padding --skip-update-check $espElf (Join-Path $out "v4-$name-merged.bin") *> (Join-Path $out "v4-$name-image.log")
    if($LASTEXITCODE -ne 0){throw 'ESP image generation failed'}
    & $xtensaObjdump -h $espElf > (Join-Path $out "v4-$name-layout.log")
}
& cargo +stable build -p tulle-t114-phy --release --features contact-store-bench --target thumbv7em-none-eabihf --offline *> (Join-Path $out 't114-build.log')
if($LASTEXITCODE -ne 0){throw 'T114 build failed'}
& 'C:\Program Files\LLVM\bin\llvm-objcopy.exe' -O binary $armElf (Join-Path $out 't114-raw.bin')
if($LASTEXITCODE -ne 0){throw 'T114 extraction failed'}
& 'C:\Program Files\LLVM\bin\llvm-objdump.exe' -h $armElf > (Join-Path $out 't114-layout.log')
$python=Join-Path $repo 'validation/results/sennet-tucket-radios/stock-cli/Scripts/python.exe'
@'
from pathlib import Path
import hashlib,json,subprocess
p=Path('validation/results/tucket-firmware-contacts/framing-stack')
for name in ['live']:
 data=(p/f'v4-{name}-merged.bin').read_bytes()[65536:]
 assert data[0]==0xe9 and len(data)<=0xf0000
 (p/f'v4-{name}-app.bin').write_bytes(data)
data=(p/'t114-raw.bin').read_bytes();data+=bytes([255])*(-len(data)%4)
assert 0x26000+(len(data)+4095)//4096*4096<=0xe4000
(p/'t114-aligned.bin').write_bytes(data)
manifest={x.name:dict(bytes=x.stat().st_size,sha256=hashlib.sha256(x.read_bytes()).hexdigest()) for x in p.glob('*.bin')}
(p/'images.json').write_text(json.dumps(manifest,indent=2)+'\n')
paths=subprocess.check_output(['git','ls-files','Cargo.toml','Cargo.lock','crates/radio-hand','crates/tucket/src','crates/selvage/src','firmware/heltec-v4-phy','firmware/t114-phy','firmware/contact_store_bench.rs'],text=True).splitlines()
sources={s:hashlib.sha256(Path(s).read_bytes()).hexdigest() for s in paths if Path(s).is_file()}
(p/'source-sha256.json').write_text(json.dumps(sources,indent=2)+'\n')
'@ | & $python -B -
if($LASTEXITCODE -ne 0){throw 'Image bounds failed'}
& C:\Users\mark_\AppData\Local\hermes\hermes-agent\venv\Scripts\adafruit-nrfutil.exe dfu genpkg --application (Join-Path $out 't114-aligned.bin') --application-version 1 --dev-type 0x52 (Join-Path $out 't114-contacts.zip') *> (Join-Path $out 't114-package.log')
if($LASTEXITCODE -ne 0){throw 'T114 package failed'}
if((git -C $repo rev-parse HEAD) -ne $env:RETINUE_FIRMWARE_REVISION -or (git -C $repo status --porcelain)){throw 'Source changed while building.'}
