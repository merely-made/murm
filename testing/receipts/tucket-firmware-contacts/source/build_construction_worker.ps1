$ErrorActionPreference='Stop'
$repo='C:\Users\mark_\Code\repos\retinue'
$out=Join-Path $repo 'validation/results/tucket-firmware-contacts/construction-worker'
if(Test-Path $out){throw 'Preserve earlier build cohort.'}
$inputs=@('Cargo.toml','Cargo.lock','crates','firmware','apps/signalman-desktop/examples/tucket_firmware_contacts.rs')
git -C $repo diff --quiet HEAD -- @inputs
if($LASTEXITCODE -ne 0){throw 'Require committed build inputs; receipt/docs work may remain pending.'}
New-Item -ItemType Directory $out|Out-Null
$env:CARGO_HOME='C:\t\cargo-homes\mere-upstreams'
$env:CARGO_TARGET_DIR=Join-Path $repo 'target'
$env:CARGO_BUILD_BUILD_DIR=Join-Path $repo 'target/build'
$env:CARGO_NET_OFFLINE='true'
$env:RETINUE_FIRMWARE_REVISION=(git -C $repo rev-parse HEAD)
[IO.File]::WriteAllText((Join-Path $out 'source-revision.txt'),$env:RETINUE_FIRMWARE_REVISION)
& cargo +esp build -p tulle-heltec-v4-phy --release --features resident-contact-store,memory-bench --target xtensa-esp32s3-none-elf --ignore-rust-version '-Zbuild-std=core,alloc' --offline *> (Join-Path $out 'v4-live-build.log')
if($LASTEXITCODE -ne 0){throw 'V4 build failed'}
$esp=Join-Path $repo 'validation/results/sennet-tucket-radios/artifacts/espflash.exe'
$elf=Join-Path $repo 'target/xtensa-esp32s3-none-elf/release/tulle-heltec-v4-phy'
& $esp save-image --chip esp32s3 --flash-size 16mb --merge --skip-padding --skip-update-check $elf (Join-Path $out 'v4-live-merged.bin') *> (Join-Path $out 'v4-live-image.log')
if($LASTEXITCODE -ne 0){throw 'Image generation failed'}
& C:\Users\mark_\.rustup\toolchains\esp\xtensa-esp-elf\bin\xtensa-esp32s3-elf-objdump.exe -h $elf > (Join-Path $out 'v4-live-layout.log')
@'
from pathlib import Path
import hashlib,json,subprocess
p=Path('validation/results/tucket-firmware-contacts/construction-worker')
data=(p/'v4-live-merged.bin').read_bytes()[65536:]
assert data[0]==0xe9 and len(data)<=0xf0000
(p/'v4-live-app.bin').write_bytes(data)
manifest={x.name:dict(bytes=x.stat().st_size,sha256=hashlib.sha256(x.read_bytes()).hexdigest()) for x in p.glob('*.bin')}
(p/'images.json').write_text(json.dumps(manifest,indent=2)+'\n')
paths=subprocess.check_output(['git','ls-files','Cargo.toml','Cargo.lock','crates/radio-hand','crates/tucket/src','crates/selvage/src','firmware/heltec-v4-phy','firmware/t114-phy','firmware/contact_store_bench.rs'],text=True).splitlines()
sources={s:hashlib.sha256(Path(s).read_bytes()).hexdigest() for s in paths if Path(s).is_file()}
(p/'source-sha256.json').write_text(json.dumps(sources,indent=2)+'\n')
'@ | & (Join-Path $repo 'validation/results/sennet-tucket-radios/stock-cli/Scripts/python.exe') -B -
if($LASTEXITCODE -ne 0){throw 'Source/image capture failed'}
git -C $repo diff --quiet HEAD -- @inputs
if($LASTEXITCODE -ne 0 -or (git -C $repo rev-parse HEAD) -ne $env:RETINUE_FIRMWARE_REVISION){throw 'Build inputs changed.'}
