"""Freeze public qualification data; retain private flash/wallet inputs ignored."""
from pathlib import Path
import hashlib
import json
import shutil
import subprocess
import re
import zlib

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
DEST = REPO / 'testing/receipts/tucket-firmware-contacts'
TRIAL = Path('final-qualified/rf-final')
load = lambda p: json.loads((ROOT / p).read_text(encoding='utf-8-sig'))
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
assert not (DEST / 'qualification.json').exists()
v4 = load('ingress-memory/storage-boxed/v4-storage.json')
t114 = load('framing-stack/t114-storage.json')
radio = load(TRIAL / 'radio.json')
assert v4['passed'] and t114['passed'] and radio['passed']
assert radio['custodian_stopped'] and radio['peer_restored'] and radio['dut_restored']
assert len(radio['phases']) == 2
for phase, count in [('first', 0), ('cold', 1)]:
    configured = load(TRIAL / phase / 'configured.json')
    result = load(TRIAL / phase / 'receipt.json')
    peer = load(TRIAL / f'{phase}-peer/receipt.json')
    assert result['passed'] and peer['result'] == 'passed'
    assert peer['ui_peer']['matched_protocol_ack'] is True
    assert result['memory']['min_stack_bytes'] == 16384
    assert result['memory']['min_free_heap_percent'] == 25
    assert configured['cached_before_peer_work'] == count
    assert 'resident air rx_ok=0 rx_damaged=0 rx_err=0 tx_ok=0 tx_err=0' in configured['status']
assert not (ROOT / TRIAL / 'cold-stock-advertise.json').exists()
source = load('final-qualified/source-sha256.json')
for path, expected in source.items():
    assert sha(REPO / path) == expected, f'changed source: {path}'
for path in ['apps/signalman-desktop/examples/tucket_firmware_contacts.rs',
             'testing/protocol-capacity/board_probe.py', 'testing/device_names.py',
             'testing/stock_tucket_bench.py', 'Cargo.toml', 'Cargo.lock',
             'apps/signalman-desktop/Cargo.toml', 'apps/signalman-desktop/Cargo.lock']:
    source[path] = sha(REPO / path)
for path in (REPO / 'crates/tucket/tests').rglob('*.rs'):
    source[path.relative_to(REPO).as_posix()] = sha(path)
source['crates/tucket/Cargo.toml'] = sha(REPO / 'crates/tucket/Cargo.toml')

def copy(src, dst):
    target = DEST / dst
    assert not target.exists(), target
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(ROOT / src, target)

# Readable reports and tool output contain public keys/status only. Never copy
# flash binaries, ZIP packages, generated wallets or raw serial input buffers.
for directory in ['corrected', 'framing-stack', 'setup-trace', 'construction-worker',
                  'ingress-memory', 'retry-budget', 'cache-reset-after-worker',
                  'cache-reset-verification', 'cache-reset-after-ingress',
                  'final-qualified', 'cache-reset-before-final']:
    for path in sorted((ROOT / directory).rglob('*')):
        if not path.is_file() or path.suffix not in {'.json', '.log', '.txt'}:
            continue
        relative = path.relative_to(ROOT)
        if 'custodian-private' in relative.parts or 'private' in path.name:
            continue
        copy(relative, Path('physical') / relative)
for name in ['preparation.json', 'prepare.log', 'wake-regression-before.log',
             'images.json', 'toolchain-and-production.json',
             'framing-fixed-tests.log', 'framing-fixed-clippy.log',
             'probe-framing-build.log', 'probe-framing-clippy.log',
             'probe-draining-build.log', 'probe-draining-clippy.log',
             'probe-trace-build.log', 'probe-trace-clippy.log',
             'retry-budget-tests.log', 'retry-budget-clippy.log',
             'final-radio-hand-tests.log', 'unsafe-final.log', 'flash-final.log',
             'production-v4-build.log', 'production-v4-layout.log',
             'production-t114-build.log', 'production-t114-layout.log', 'loc-final.log',
             'production-v4-final-build.log', 'production-v4-final-layout.log',
             'production-t114-final-build.log', 'production-t114-final-layout.log',
             'flash-audit.log', 'flash-self-test.log', 'unsafe.log', 'loc-extracted.log',
             'format-final.log', 'validation-self-test.log']:
    copy(name, Path('software') / name)
for name in ['prepare_boards.py', 'prepare_corrected.py', 'build_pinned.ps1',
             'build_framing_stack.ps1', 'v4_storage_campaign.py',
             'v4_storage_retry.py', 't114_storage_campaign.py',
             't114_storage_worker.py', 'live_ops.py', 'live_ops_framing.py',
             'live_ops_trace.py', 'live_ops_worker.py', 'live_ops_ingress.py', 'live_ops_retry.py',
             'live_ops_final.py', 'build_final_qualified.ps1', 'run_radio_final.ps1', 'reset_owned_final_cache.py',
             'build_setup_trace.ps1', 'build_construction_worker.ps1', 'build_ingress_memory.ps1', 'build_retry_budget.ps1',
             'run_radio.ps1', 'run_radio_framing.ps1', 'run_radio_trace.ps1', 'run_radio_worker.ps1',
             'run_radio_ingress.ps1', 'run_radio_retry.ps1', 'v4_storage_boxed.py',
             'reset_owned_bench_cache.py', 'reset_owned_ingress_cache.py', 'verify_owned_cache_reset.py',
             'freeze_firmware_contacts.py']:
    copy(name, Path('source') / name)
# Discovery includes these new frozen sources before the ledger is sealed.
with (ROOT / 'registry-final.log').open('w') as log:
    checked = subprocess.run([str(REPO / 'validation/results/sennet-tucket-radios/stock-cli/Scripts/python.exe'),
                              '-B', 'validation/run.py', 'verify'], cwd=REPO, stdout=log, stderr=subprocess.STDOUT)
assert checked.returncode == 0
copy('registry-final.log', Path('software/registry-final.log'))
(DEST / 'source-sha256.json').write_text(json.dumps(source, indent=2) + '\n')

# Parse only the public contact record out of the retained private tail.
tail = (ROOT / TRIAL / 'sb2-live-private-tail.bin').read_bytes()
cache = tail[0xa000:0xc000]
before = re.findall(r'^0x([0-9a-fA-F]{1,32})\s*$',
                    (ROOT / TRIAL / 'dut-cold-reset.log').read_text(), re.MULTILINE)
assert len(before) == 1 and before[0].lower().zfill(32) == hashlib.md5(cache).hexdigest()
page = cache[:4096]
assert cache[4096:] == bytes([255]) * 4096 and page[:6] == b'RHS0\x01\x00'
length = int.from_bytes(page[6:8], 'little'); body = page[16:16+length]
assert zlib.crc32(page[:12] + body) == int.from_bytes(page[12:16], 'little')
assert body[:8] == b'TKC0\x01\x00\x01\x00' and len(body) == 104
assert body[8:40].hex() == radio['self_public'] and body[72:104].hex() == radio['peer_public']
sequence = int.from_bytes(page[8:12], 'little'); assert sequence == 0
(DEST / 'public-cache-readback.json').write_text(json.dumps(dict(
    before_reset_and_final_md5=hashlib.md5(cache).hexdigest(), unchanged_after_cold_traffic=True,
    slot='A', sequence=sequence, local_public=body[8:40].hex(),
    receive_profile_digest=body[40:72].hex(), keys=[body[72:104].hex()]), indent=2) + '\n')

roots = ['validation/results/firmware-memory', 'validation/results/tucket-contacts',
         'validation/results/tucket-firmware-contacts',
         'testing/receipts/protocol-parity-memory', 'testing/receipts/tucket-contacts']
retained = {p: sum(f.stat().st_size for f in (REPO / p).rglob('*') if f.is_file())
            for p in roots}
report = dict(schema=1, result='passed_bounded_standalone_public_contact_store',
              latest_source=radio['source'].strip(),
              probe_source=radio['probe_source'].strip(),
              source_binding='committed build inputs per cohort; receipt/index edits were pending in later builds; source-sha256.json binds latest firmware and host runner',
              cohorts={'9841f3e7ed6': 'preflight caught packet-lease overlap before any firmware write',
                       '30bde0d1ab9': 'corrected allocation, eight-key V4 reset/memory pass; T114 stack and live USB setup failures retained',
                       '5f7295c1ac8': 'T114 separate-turn fixture and fragmented USB setup fix; latest firmware physical qualification',
                       '2367ae37355': 'continuous host draining did not resolve setup; failure retained',
                       '6d27e711a61': 'public stage trace isolated constructor stall; nested frames exceeded the available stack',
                       '096283afb94': 'separate construction worker and boxed runtime; contact save and outbound ACK pass, incoming return fails',
                       '2a4e654fbe6': 'boxed storage charged during memory pressure; RX metadata, no return-text frame captured',
                       '21ac0176a22': 'radio-budget retry floor and delayed-ACK regression; first full live reboot qualification',
                       'fb7e0e6eacc': 'constructor owns public error formatter; final live source respects main LOC baseline'},
              storage=dict(max_keys=8, record_bytes=344,
                           v4_range=['0x3fa000', '0x3fc000'],
                           t114_range=['0xe4000', '0xe6000'],
                           commit_magic_last=True, scope='local public identity and exact receive profile',
                           payload='public peer keys only; pseudonymous associations'),
              software=dict(tucket=84, radio_hand_lib=150, instances=19, contact_journal=7,
                            word_cut_boundaries_one_and_two_keys=68, usb_chunk_widths=list(range(1,65)),
                            core_clippy=True, host_probe_clippy=True, format=True,
                            flash_classification=True, unsafe_audit=True,
                            loc='two existing application failures; software/loc-extracted.log'),
              physical=dict(v4_storage='physical/ingress-memory/storage-boxed/v4-storage.json',
                            t114_storage='physical/framing-stack/t114-storage.json',
                            radio=(Path('physical') / TRIAL / 'radio.json').as_posix(),
                            texts=4, protocol_acks=4, dut_boots=2,
                            restored_contacts_before_peer_work=[0,1],
                            explicit_peer_advert_after_dut_reset=False,
                            min_stack_bytes=16384, min_free_heap_percent=25),
              limits=['V4 live resident storage and T114 storage backend only; T114 has no Tucket service loop',
                      'volatile identity/setup must be supplied again after boot; no credential persistence',
                      'word-cut model plus physical reset/reflash; no battery-disconnected power-cut qualification',
                      'stock RAM-only app under original factory partition table; stock persistence not qualified',
                      'no new desktop UI, multihop/role or unattended-product acceptance'],
              retention=dict(roots_bytes=retained, cap_bytes=100000000,
                             private_owner='this receipt: ignored recovery images, fresh protected-state readbacks and stopped isolated Keeper wallets',
                             isolated_targets_homes_worktrees_created=False))
(DEST / 'qualification.json').write_text(json.dumps(report, indent=2) + '\n')
files = [dict(path=p.relative_to(DEST).as_posix(), bytes=p.stat().st_size, sha256=sha(p))
         for p in sorted(DEST.rglob('*')) if p.is_file() and p.name != 'files.json']
(DEST / 'files.json').write_text(json.dumps(files, indent=2) + '\n')
public_bytes = sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file())
assert sum(retained.values()) + public_bytes < 100000000
print(json.dumps(dict(files=len(files), public_bytes=public_bytes,
                      combined_retained_bytes=sum(retained.values()) + public_bytes)))
