"""Freeze the completed public contact trial without private recovery inputs."""
from pathlib import Path
import hashlib
import json
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
DEST = REPO/'testing/receipts/tucket-contacts'
assert not (DEST/'qualification.json').exists()
load = lambda path: json.loads((ROOT/path).read_text(encoding='utf-8-sig'))
sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
trial = load('native-trial.json')
assert trial['passed'] and trial['custodian_stopped'] and trial['peer_restored'] and trial['dut_restored']
for path, expected in trial['source_sha256'].items():
    assert sha(REPO/path) == expected.lower(), f'source changed after physical run: {path}'
native = load('native/run.json')
assert native['result'] == 'completed_pending_visual_review' and native['require_cached_contacts']
assert not (ROOT/'native/stock-reconnect-advertise.json').exists()
for folder in ('peer','peer-reconnect'):
    peer = load('native/'+folder+'/receipt.json')
    assert peer['result'] == 'passed'
    assert peer['ui_peer']['matched_protocol_ack'] is True
book = load('native/desktop-journal.json')
assert book['version'] == 2 and len(book['records']) == 4
assert [bytes(key).hex() for key in book['meshcore_contacts']] == [native['peer_address']]
assert len({r['message']['id'] for r in book['records']}) == 4
assert sum(r['delivery'] == 'ProtocolAcknowledged' for r in book['records']) == 2
assert sum(r['delivery'] == 'ReceivedByRadio' for r in book['records']) == 2
assert load('native/stock-advertise.json')['before']['key'] == native['peer_address']
assert 'RESULT ok' in (ROOT/'native/scenario.done').read_text()

def copy(src, dst=None):
    target = DEST/(dst or src)
    assert not target.exists(), target
    target.parent.mkdir(parents=True,exist_ok=True)
    shutil.copyfile(ROOT/src,target)

for name in ('format','full-tucket-tests','full-signalman-tests','radio-hand-tests',
             'final-clippy','thumb-check','xtensa-check','desktop-final-build',
             'desktop-clippy','unsafe','extracted-loc'):
    copy(name+'.log','software/'+name+'.log')
for name in ('preflight-refusal.json','native-trial.json','rollback.json',
             'sb1-before.json','sb1-restored.json','sb1-restoration.json',
             'sb2-before.json','sb2-restored.json','dut-profile.json','dut-profile-restored.json',
             'public-card.json','public-config.json','stock-setup.json'):
    copy(name,'physical/'+name)
for name in ('sb1-backup-prefix-md5','sb1-backup-tail-md5','sb1-install-precheck',
             'stock-app-install','sb1-app-restore','sb1-data-before-restore',
             'sb1-restored-prefix-md5','sb1-restored-tail-md5'):
    copy(name+'.log','physical/'+name+'.log')
for name in ('run.json','scenario.done','companion.scn','desktop-journal.json',
             'tucket-config.json','stock-advertise.json','desktop.stderr.log',
             'desktop.stdout.log','peer.stdout.log','peer.stderr.log',
             'peer-reconnect.stdout.log','peer-reconnect.stderr.log',
             'peer/receipt.json','peer-reconnect/receipt.json'):
    copy('native/'+name)
captures = ['form','connected','roundtrip','disconnected','reconnected','reconnect-roundtrip','stopped']
for name in captures:
    copy('native/'+name+'.png')
for name in ('stock_app_trial.py','run_cached_contacts.ps1','freeze_contacts.py'):
    copy(name,'source/'+name)

source = dict(trial['source_sha256'])
extra = ['Cargo.toml','Cargo.lock','apps/signalman/Cargo.toml',
         'apps/signalman-desktop/Cargo.toml','apps/signalman-desktop/Cargo.lock',
         'crates/tucket/tests/persistent_contacts.rs','testing/run_tucket_desktop.ps1',
         'testing/stock_tucket_bench.py','testing/device_names.py']
for path in extra:
    source[path] = sha(REPO/path)
(DEST/'source-sha256.json').write_text(json.dumps(source,indent=2)+'\n')
visual = dict(result='functional_pass',reviewer='Codex visual inspection',
              captures=captures,dimensions=[2400,1504],blank=0,distinct=7,
              observations=['Commissioned COM7 and explicit realm are visible.',
                            'Incoming text and protocol-acknowledged outgoing text are visible before and after reconnect.',
                            'Disconnected/reconnected captures preserve old history; scenario asserts exact records.',
                            'Existing stale connecting notice, horizontally cropped fields and final four-record viewport clipping remain cosmetic open work.'])
(DEST/'visual-review.json').write_text(json.dumps(visual,indent=2)+'\n')

private_bytes = sum(p.stat().st_size for p in ROOT.rglob('*') if p.is_file())
memory_bytes = sum(p.stat().st_size for p in (REPO/'validation/results/firmware-memory').rglob('*') if p.is_file())
memory_public = sum(p.stat().st_size for p in (REPO/'testing/receipts/protocol-parity-memory/closure').rglob('*') if p.is_file())
report = dict(schema=1,result='passed_bounded_host_contacts',
              native_base_revision=trial['source'],
              source_binding='base revision plus exact tested source hashes; native run used a dirty source tree, not a clean-source receipt',
              source_manifest='source-sha256.json',
              authority=trial['authority'],custodian_sha256=trial['helper_sha256'],
              supplier_qualification='testing/receipts/sennet-desktop/custody-supplier.json',
              software=dict(tucket=83,signalman=75,radio_hand=168,format=True,
                            core_clippy=True,desktop_clippy=True,desktop_build=True,
                            thumb_no_std=True,xtensa_no_std=True,unsafe_audit=True,
                            loc='three existing unrelated failures retained in software/extracted-loc.log'),
              native=dict(texts=4,matched_protocol_acks=4,connections=2,
                          explicit_initial_peer_advert=True,explicit_reconnect_peer_advert=False,
                          exact_history=True,final_public_contacts=1,visual_review='visual-review.json'),
              restoration=dict(sb1='fresh 1 MiB prefix and protected 64 KiB tail match ROM MD5; exact named firmware/status/settings readback',
                               sb2='same-boot profile restoration acknowledged; original named firmware/status/settings exact',
                               t114='untouched in contact continuation',owned_helpers_stopped=True),
              limits=['host-owned public-key cache, not standalone MCU flash persistence',
                      'unit cold reload proves absence of fresh advert; native fixture omits its reconnect advert, not an RF-wide claim of zero adverts',
                      'stock application-only reference under original factory table has no SPIFFS partition; stock reboot/filesystem persistence not qualified',
                      'no power-cut, personal installed Djinn daemon, broader routing/roles or UI polish acceptance'],
              retained_inputs=dict(contact_bytes_before_freeze=private_bytes,memory_input_bytes=memory_bytes,
                                   memory_closure_public_bytes=memory_public,total_cap_bytes=100000000,
                                   private_recovery='ignored validation/results/tucket-contacts; fresh flash backups and stopped synthetic Keeper wallet',
                                   isolated_targets_homes_worktrees_created=False))
(DEST/'qualification.json').write_text(json.dumps(report,indent=2)+'\n')
files = [{'path':p.relative_to(DEST).as_posix(),'bytes':p.stat().st_size,'sha256':sha(p)}
         for p in sorted(DEST.rglob('*')) if p.is_file() and p.name != 'files.json']
(DEST/'files.json').write_text(json.dumps(files,indent=2)+'\n')
public_bytes = sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file())
assert private_bytes+memory_bytes+memory_public+public_bytes < 100000000
print(json.dumps(dict(files=len(files),public_bytes=public_bytes,
                      combined_retained_bytes=private_bytes+memory_bytes+memory_public+public_bytes)))
