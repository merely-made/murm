"""Reset only this lane's newly created, identified synthetic SB2 cache pair.

The retained fresh tail proves the exact preimage. No settings, control or
monotonic reservation sector is erased or rolled back. This is an attended ROM
bench action, not a production cache-clear command.
"""
from pathlib import Path
import hashlib,json,re,subprocess,sys,time,zlib
from serial.tools.list_ports import comports
base=Path(__file__).resolve().parent;repo=base.parents[2]
prior=base/'construction-worker/rf-worker'
out=base/'cache-reset-after-worker';assert not out.exists();out.mkdir()
trial=json.loads((prior/'radio.json').read_text(encoding='utf-8-sig'))
assert trial['custodian_stopped'] and trial['dut_restored'] and trial['peer_restored']
restore=json.loads((prior/'sb2-live-restoration.json').read_text())
tail=(prior/'sb2-live-private-tail.bin').read_bytes();assert len(tail)==65536
assert hashlib.md5(tail).hexdigest()==restore['tail_md5']
origin=0xa000;end=0xc000
original=(base/'corrected/sb2-private-tail.bin').read_bytes()
assert original[origin:end]==bytes([255])*8192
records=[]
for offset in [origin,origin+4096]:
 page=tail[offset:offset+4096]
 if page==bytes([255])*4096:continue
 assert page[:6]==b'RHS0\x01\x00'
 size=int.from_bytes(page[6:8],'little');body=page[16:16+size]
 assert zlib.crc32(page[:12]+body)==int.from_bytes(page[12:16],'little')
 assert body[:8]==b'TKC0\x01\x00\x01\x00' and len(body)==104
 assert body[8:40].hex()==trial['self_public'] and body[72:104].hex()==trial['peer_public']
 records.append(int.from_bytes(page[8:12],'little'))
assert records==[0],records
tool=repo/'validation/results/sennet-tucket-radios/artifacts/espflash.exe'
assert hashlib.sha256(tool.read_bytes()).hexdigest()=='0cc03364c70a86325236f18ad1aaed17eedf267d89312c0cdabe4964f5cb758e'
def rom(name,mode,*args):
 assert any((p.device,p.vid,p.pid,p.serial_number)==('COM7',0x303a,0x1001,'44:1B:F6:6A:FA:64') for p in comports())
 with (out/(name+'.log')).open('x') as log:
  r=subprocess.run(list(map(str,[tool,mode,'--port','COM7','--chip','esp32s3','--before','usb-reset','--after','watchdog-reset','--baud','115200','--non-interactive','--skip-update-check',*args])),stdout=log,stderr=subprocess.STDOUT,timeout=300)
 time.sleep(6);assert r.returncode==0,name
 return (out/(name+'.log')).read_text()
def checksum(name,address,size):
 values=re.findall(r'0x([0-9a-fA-F]{32})',rom(name,'checksum-md5',address,size));assert len(values)==1
 return values[0].lower()
result={'passed':False,'scope':'only lane-owned synthetic contact pair; retained raw preimage, no nonce rollback',
        'prior':'construction-worker/rf-worker','local_public':trial['self_public'],
        'peer_public':trial['peer_public'],'erase_range':['0x3fa000','0x3fc000']}
try:
 assert checksum('before-tail',0x3f0000,65536)==restore['tail_md5']
 assert checksum('before-prefix',0,1048576)==restore['prefix_md5']
 rom('erase-owned-contact-pair','erase-region',0x3fa000,8192)
 expected=bytearray(tail);expected[origin:end]=bytes([255])*8192
 result['after_tail_md5']=checksum('after-tail',0x3f0000,65536)
 assert result['after_tail_md5']==hashlib.md5(expected).hexdigest()
 assert checksum('after-prefix',0,1048576)==restore['prefix_md5']
 result['passed']=True
finally:
 (out/'reset.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
