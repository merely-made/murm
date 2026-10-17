from pathlib import Path
import hashlib,json,re,subprocess,time
from serial.tools.list_ports import comports
b=Path('validation/results/tucket-firmware-contacts');p=b/'cache-reset-after-worker';q=b/'cache-reset-verification';assert not q.exists();q.mkdir()
r=json.loads((b/'construction-worker/rf-worker/sb2-live-restoration.json').read_text())
tail=bytearray((b/'construction-worker/rf-worker/sb2-live-private-tail.bin').read_bytes());tail[0xa000:0xc000]=bytes([255])*8192
expected=hashlib.md5(tail).hexdigest();old=(p/'after-tail.log').read_text().strip()
assert old.startswith('0x') and old[2:].zfill(32).lower()==expected
print('expected full tail MD5:',expected)
tool=b.parent/'sennet-tucket-radios/artifacts/espflash.exe'
assert hashlib.sha256(tool.read_bytes()).hexdigest()=='0cc03364c70a86325236f18ad1aaed17eedf267d89312c0cdabe4964f5cb758e'
def md5(name,address,size):
 assert any((v.device,v.vid,v.pid,v.serial_number)==('COM7',0x303a,0x1001,'44:1B:F6:6A:FA:64') for v in comports())
 with (q/(name+'.log')).open('x') as f:
  x=subprocess.run(list(map(str,[tool,'checksum-md5','--port','COM7','--chip','esp32s3','--before','usb-reset','--after','watchdog-reset','--baud','115200','--non-interactive','--skip-update-check',address,size])),stdout=f,stderr=subprocess.STDOUT,timeout=120)
 time.sleep(6);assert x.returncode==0
 matches=re.findall(r'^0x([0-9a-fA-F]{1,32})\s*$',(q/(name+'.log')).read_text(),re.MULTILINE);assert len(matches)==1
 return matches[0].lower().zfill(32)
report={'passed':False,'mutation':'none; read-only follow-up to completed bench-owned cache erase','prior_harness_failure':'ROM MD5 omits leading zero; first parser required 32 printed digits'}
try:
 report['tail_md5']=md5('tail',0x3f0000,65536);assert report['tail_md5']==expected
 report['prefix_md5']=md5('prefix',0,1048576);assert report['prefix_md5']==r['prefix_md5']
 report['passed']=True
finally:
 (q/'verification.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
