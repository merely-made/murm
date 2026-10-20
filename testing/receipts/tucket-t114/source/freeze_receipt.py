"""Seal only passing public T114 evidence; never copy wallets or flash dumps."""
from pathlib import Path
import hashlib,json,re,shutil,subprocess
REPO=Path(r'C:\Users\mark_\Code\repos\retinue')
BASE=REPO/'validation/results/tucket-t114'
OUT=REPO/'testing/receipts/tucket-t114'
read=lambda p:json.loads(p.read_text(encoding='utf-8-sig'))
radio=read(BASE/'radio/radio.json')
assert radio['passed'] and not radio['custodian_retained_for_owned_retry']
assert read(BASE/'custody-controls/qualification.json')['result']=='passed'
assert read(BASE/'fixture-handoff/receipt.json')['passed']
board=read(BASE/'quiet-pressure/board.json')
assert board['passed'] and board['requested_rounds']==16
assert read(BASE/'quiet-pressure-restoration/receipt.json')['passed']
assert read(BASE/'radio/dut-restoration/receipt.json')['passed']
assert read(BASE/'radio/sb1-live-restoration.json')['passed']
source=(BASE/'quiet-images/source-revision.txt').read_text().strip()
assert radio['firmware_source']==source and f'build={source}' in board['attached']
measurements=[dict(re.findall(r'(\w+)=([0-9]+)',r['response'])) for r in board['responses'] if r['command']=='protocol memory']
assert len(measurements)==16
assert all(m['initial']==m['final']=='0' and int(m['failures'])==i for i,m in enumerate(measurements,1))
results=[]
for phase,expected in [('first',0),('cold',1)]:
    receipt=read(BASE/f'radio/{phase}/receipt.json')
    configured=read(BASE/f'radio/{phase}/configured.json')
    peer=read(BASE/f'radio/{phase}-peer/receipt.json')
    assert receipt['passed'] and receipt['memory']['passed'] and peer['result']=='passed'
    assert configured['cached_before_peer_work']==expected
    if phase=='cold': assert 'Tucket(Advert {' not in '\n'.join(receipt['transcript'])
    records=peer['left']['bench_records']
    incoming=[r for r in records if r['message']['direction']=='Incoming']
    outgoing=[r for r in records if r['message']['direction']=='Outgoing' and r['delivery']=='ProtocolAcknowledged']
    assert len(incoming)==len(outgoing)==1
    tag=incoming[0]['message']['body']['PlainText']
    assert outgoing[0]['message']['body']['PlainText']==tag+'-return'
    assert incoming[0]['message']['author']['address']==radio['self_public']
    assert receipt['protocol_acknowledged'] and receipt['received_exact_return_text']
    assert all('heap_alloc_failures=0' in t for t in [receipt['transcript'][-1]])
    results.append(dict(phase=phase,cached_before_peer_work=expected,texts=2,protocol_acks=2,memory=receipt['memory']))
assert not OUT.exists()
OUT.mkdir(parents=True)
def copy(src,dest=None):
    src=BASE/src
    dest=OUT/(dest or str(src.relative_to(BASE)))
    dest.parent.mkdir(parents=True,exist_ok=True)
    shutil.copyfile(src,dest)
for folder in ['radio','quiet-pressure','quiet-pressure-restoration','fixture-handoff','custody-controls']:
    for path in (BASE/folder).rglob('*'):
        rel=path.relative_to(BASE)
        if path.is_file() and path.suffix in ('.json','.log') and not any(s in rel.parts for s in ('runtime','custodian-private')) and 'private' not in path.name:
            copy(rel)
for folder in ['quiet-images','images']:
    for path in (BASE/folder).iterdir():
        if path.suffix in ('.json','.txt','.log'):
            copy(path.relative_to(BASE))
for name in ['t114-before.json','probe-build-online.log','tests-full.log','clippy-final.log','tucket-tests.log','unsafe.log','format.log','inventory-self-test.log','flash-classification.log','loc.log']:
    copy(Path(name),'software/'+name)
for name in ['public-config.json','public-card.json']:
    copy(Path('custody')/name,Path('custody')/name)
for name in ['run_radio.ps1','v4_ops.py','run_custody.ps1','build_quiet_images.ps1','run_quiet_pressure.ps1']:
    copy(Path(name),'source/'+name)
copy(Path('freeze_receipt.py'),'source/freeze_receipt.py')
(OUT/'.gitattributes').write_text('* -text whitespace=cr-at-eol,-blank-at-eof,-blank-at-eol\n')
paths=subprocess.check_output(['git','ls-files','Cargo.toml','Cargo.lock','crates/radio-hand','crates/tucket/src','firmware/t114-phy','firmware/contact_store_bench.rs','testing/protocol-capacity','testing/tucket_t114_ops.py','apps/signalman-desktop/examples/tucket_firmware_contacts.rs','apps/signalman-desktop/Cargo.toml','apps/signalman-desktop/Cargo.lock'],cwd=REPO,text=True).splitlines()
sources={p:hashlib.sha256((REPO/p).read_bytes()).hexdigest() for p in paths}
(OUT/'source-sha256.json').write_text(json.dumps(sources,indent=2)+'\n')
qualification=dict(passed=True,firmware_source=source,host_probe_source_sha256=radio['probe_source_sha256'],instrumented_only=True,board='TULLE-T114-01',peer='SB1 / MeshCore 1.17.1 d929643',min_stack_bytes=16384,min_free_heap_percent=25,rounds=16,min_untouched_stack=min(int(m['stack_free']) for m in measurements),max_heap=max(int(m['peak']) for m in measurements),heap_capacity=49152,zero_final_heap=True,expected_refusals=16,phases=results,texts=4,protocol_acks=4,original_firmware_identity_profiles_restored=True,scope='one-hop Tucket endpoint after volatile scoped USB commissioning; contacts survive software reset without fresh peer advert',limits=['private setup remains volatile','battery-connected software reset; no power-cut qualification','no installed personal daemon, UI, repeater/server or broader routing claim'],target_policy='existing shared repository targets and Cargo homes reused; no isolated target/home/worktree created')
(OUT/'qualification.json').write_text(json.dumps(qualification,indent=2)+'\n')
roots=[BASE,REPO/'validation/results/tucket-firmware-contacts',REPO/'validation/results/firmware-memory',REPO/'validation/results/tucket-contacts',OUT]
sizes={str(p.relative_to(REPO)):sum(f.stat().st_size for f in p.rglob('*') if f.is_file()) for p in roots}
assert sum(sizes.values())<100_000_000
(OUT/'retained-bytes.json').write_text(json.dumps(dict(cap=100_000_000,owned_receipt_input_bytes=sizes,total=sum(sizes.values()),note='borrowed installed tools and reusable Cargo caches are not copied into the receipt'),indent=2)+'\n')
print(json.dumps(qualification,indent=2))