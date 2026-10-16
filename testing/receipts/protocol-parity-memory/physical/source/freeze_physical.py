"""Seal the physical checkpoint without altering any failed raw report."""
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess

root=Path.cwd()
work=root/'validation/results/protocol-parity-memory'
out=root/'testing/receipts/protocol-parity-memory/physical'
assert not (out/'qualification.json').exists()
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
read=lambda n:json.loads((work/n).read_text())
spec=importlib.util.spec_from_file_location('probe',root/'testing/protocol-capacity/board_probe.py')
probe=importlib.util.module_from_spec(spec); spec.loader.exec_module(probe)
v4=read('v4-peer-campaign.json'); arm=read('t114-worker-campaign.json')
report={'base_revision':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
        'result':'bounded_target_heap_campaigns_and_separate_restoration_checks_passed',
        'phase_2_complete':False,'worst_case_stack_qualified':False,'reserve_margin_selected':False,
        'boards':{},'restoration':{},'source_sha256':{},'runners':{}}
for family,campaign,capacity in [('v4',v4,65536),('t114',arm,49152)]:
    rows=[r for r in campaign['responses'] if r['command']=='protocol memory']
    assert len(rows)==16
    for index,row in enumerate(rows,1):
        m={k:(v if v in ('true','false') else int(v)) for k,v in row['measurement'].items()}
        assert [m[k] for k in ['initial','fragmented','retained','final','peak','capacity','failures','expected_refusals']]==[0,4544,14476,0,18452,capacity,index,1]
    statuses=[r['response'] for r in campaign['responses'] if r['command']=='status']
    assert len(statuses)==2 and all('phy online' in s for s in statuses)
    assert probe.canonical_status(statuses[0])==probe.canonical_status(statuses[1])
    report['boards'][family]={'heap_campaign_passed':True,'rounds':16,'capacity':capacity,
        'occupied_peak':18452,'protocol_retained_above_guards':9932,'expected_refusals':16,
        'unexpected_refusals':0,'after_each_round':0,'raw_runner_passed':campaign['passed'],
        'first_measurement':rows[0]['measurement'],'last_measurement':rows[-1]['measurement'],
        'round_seconds':[r['seconds'] for r in rows]}
assert v4['error']=="AssertionError('status changed during campaign')"
first=[r['response'] for r in v4['responses'] if r['command']=='status'][0]
report['v4_raw_failure_review']={'accepted_separately':True,'only_removed_prefix':first[:first.index('tulle/')],
 'reason':'ROM boot fragment precedes first status; all complete firmware and identity bytes match after it',
 'raw_report_unchanged':True,'diagnostic_full_flash_readback':False}
# Unknown noise, changed identities and changed reset fields must remain different.
assert probe.canonical_status('noise tulle/status')=='noise tulle/status'
assert probe.canonical_status('tulle/status identity=A')!=probe.canonical_status('tulle/status identity=B')
assert probe.canonical_status('tulle/status reset=soft')!=probe.canonical_status('tulle/status reset=watchdog')
(work/'status-parser-checks.json').write_text(json.dumps({'captured_rom_prefix_case':True,'unknown_noise_not_hidden':True,'identity_and_reset_changes_not_hidden':True},indent=2)+'\n')
for family,before_name,after_name,wrapper_name in [
 ('v4','v4-peer-probe-before.json','v4-peer-final-original-status.json','v4-peer-physical.json'),
 ('v4_com7','v4-before.json','v4-final-original-status.json','v4-watchdog-physical.json')]:
    before=read(before_name); after=read(after_name); wrapper=read(wrapper_name)
    assert after['passed'] and wrapper['original_prefix_exact'] and wrapper['protected_tail_exact']
    assert all(b['response']==a['response'] for b,a in zip(before['responses'][:4],after['responses'][:4]))
    report['restoration'][family]={'prefix_and_protected_tail_byte_exact':True,
        'fresh_original_status_identity_region_channel_sync':True,'wrapper_raw_passed':wrapper['passed'],
        'live_probe':after_name,'fresh_probe_sha256':sha(work/after_name)}
assert read('t114-worker-physical.json')['passed'] and read('t114-final-original-status.json')['passed']
assert all(s in read('t114-final-original-status.json')['attached'] for s in ['identity=loaded slot=B seq=93','node=599997c8'])
assert read('t114-worker-final-equivalence.json')['final_source_bench_binary_exactly_matches_installed']
report['restoration']['t114']={'application_only_dfu_completed':True,'original_identity_sequence_node_profile':True,
 'protected_flash_readback':False,'intentional_watchdog_bridge_changes_boot_cause_and_crash_residue':True}
report['artifacts']={n:read(n) for n in ['t114-worker-elf.json','v4-worker-elf.json','t114-handoff-elf.json','t114-worker-final-equivalence.json']}
report['runners']={n:sha(work/n) for n in ['board_probe_t114.py','board_probe_v4.py','v4_peer_physical.py','t114_worker_physical.py']}
report['gates']={'host_and_target_selected_clippy':'passed with recorded pre-existing board lint exceptions',
 'unsafe_audit':'61 approved tokens in 16 files; 22 roots','format':'passed','registry':'passed',
 'loc':'six unrelated existing violations; changed channels file reduced below ceiling'}
report['retained_outputs']={'target':'existing repository target/build',
 'cargo_home':'co-owned C:/t/cargo-homes/mere-upstreams','isolated_targets_homes_worktrees_created':False,
 'private_backups_and_images':'ignored raw physical receipt inputs retained for recovery/reproduction'}
cutoff=(work/'t114-bench-build.log').stat().st_mtime-30
selected=[]
for path in sorted(work.iterdir()):
    if not path.is_file() or path.stat().st_mtime<cutoff: continue
    if path.suffix not in ('.json','.log','.py'): continue
    if path.suffix=='.py' and not (path.name.endswith('_physical.py') or path.name.startswith('board_probe_') or path.name=='freeze_physical.py'): continue
    target=out/('source' if path.suffix=='.py' else 'evidence')/path.name
    target.parent.mkdir(exist_ok=True,parents=True)
    assert not target.exists()
    shutil.copyfile(path,target)
    selected.append(target)
(out/'.gitattributes').write_text('* -text whitespace=cr-at-eol,-blank-at-eof,-blank-at-eol\n')
manifest=root/'validation/manifest.toml'
text=manifest.read_text()
for path in selected:
    if path.suffix!='.py':continue
    rel=path.relative_to(root).as_posix()
    assert f'path = "{rel}"' not in text
    text+=f'\n[[asset_exemption]]\npath = "{rel}"\nreason = "Frozen attended physical-memory evidence source; not an automatic hardware test."\nexpires = "2027-01-16"\n'
manifest.write_text(text)
paths=set(subprocess.check_output(['git','ls-files','-z'],text=True).split('\0'))
paths.update(str(p.relative_to(root)).replace('\\','/') for parent in ['firmware/heltec-v4-phy','firmware/t114-phy','testing/protocol-capacity'] for p in (root/parent).rglob('*.rs'))
for name in sorted(paths):
    if not name:continue
    p=root/name
    if p.is_file() and (name in ['Cargo.toml','Cargo.lock','.cargo/config.toml','rust-toolchain.toml'] or
       (name.startswith(('crates/','firmware/','testing/protocol-capacity/')) and p.suffix in ('.rs','.toml','.x','.py'))):
        report['source_sha256'][name]=sha(p)
(out/'qualification.json').write_text(json.dumps(report,indent=2)+'\n')
files={p.relative_to(out).as_posix():sha(p) for p in sorted(out.rglob('*')) if p.is_file() and p.name!='files.json'}
(out/'files.json').write_text(json.dumps(files,indent=2)+'\n')
print(json.dumps({'qualified':True,'sealed_files':len(files),'source_files':len(report['source_sha256'])}))
