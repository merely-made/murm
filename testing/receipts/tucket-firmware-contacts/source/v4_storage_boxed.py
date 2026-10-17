"""SB1 before-radio full contact table and cold-reload qualification."""
from pathlib import Path
import hashlib,json,re,subprocess,sys,time
import serial
from serial.tools.list_ports import comports
base=Path(__file__).resolve().parent;p=base/"ingress-memory";private=base/"corrected";repo=base.parents[2]
tool=repo/'validation/results/sennet-tucket-radios/artifacts/espflash.exe'
assert hashlib.sha256(tool.read_bytes()).hexdigest()=='0cc03364c70a86325236f18ad1aaed17eedf267d89312c0cdabe4964f5cb758e'
assert json.loads((private/'preparation.json').read_text())['passed']
pin=json.loads((p/'images.json').read_text())['v4-bench-app.bin']['sha256']
assert hashlib.sha256((p/'v4-bench-app.bin').read_bytes()).hexdigest()==pin
report={'passed':False,'source':(p/'source-revision.txt').read_text().strip(),'app_sha256':pin,'boots':[]}
q=p/'storage-boxed';assert not q.exists();q.mkdir()
report['first_write_receipt']='../../corrected/v4-storage.json'
def identity():
 assert any((x.device,x.vid,x.pid,x.serial_number)==('COM6',0x303a,0x1001,'44:1B:F6:6A:FB:28') for x in comports())
def run(name,args,timeout=300):
 with (q/(name+'.log')).open('x') as log:
  result=subprocess.run(list(map(str,args)),stdout=log,stderr=subprocess.STDOUT,timeout=timeout)
 assert result.returncode==0,name
 return (q/(name+'.log')).read_text()
def rom(name,mode,*args):
 identity()
 try:return run(name,[tool,mode,'--port','COM6','--chip','esp32s3','--before','usb-reset','--after','watchdog-reset','--baud','115200','--non-interactive','--skip-update-check',*args])
 finally:time.sleep(6)
def checksum(name,address,size):
 result=rom(name,'checksum-md5',address,size)
 values=re.findall(r'^0x([0-9a-fA-F]{1,32})\s*$',result,re.MULTILINE);assert len(values)==1
 return values[0].lower().zfill(32)
def query(command):
 identity();link=serial.Serial(port=None,baudrate=115200,timeout=.1,write_timeout=2)
 link.port,link.dtr,link.rts='COM6',False,False
 with link:
  link.read(4096);link.write(command.encode()+b'\n');link.flush()
  response=bytearray();until=time.monotonic()+1.5
  while time.monotonic()<until:response.extend(link.read(4096))
 return response.decode(errors='replace')
try:
 report['touched']=True
 rom('v4-storage-install','write-bin',65536,p/'v4-bench-app.bin')
 first=query('contacts bench');report['boots'].append(first)
 assert 'ok initial_keys=8 keys=8 commits=0 sequence=1' in first,first
 run('v4-storage-memory',[sys.executable,'-B',repo/'testing/protocol-capacity/board_probe.py','v4','--port','COM6','--usb-serial','44:1B:F6:6A:FB:28','--rounds',16,'--min-stack-bytes','16384','--min-free-heap-percent','25','--output',q/'v4-storage-memory.json'])
 for i in range(2):
  report.setdefault('cache_md5',[]).append(checksum('v4-storage-reset-'+str(i),0x3fa000,8192))
  cold=query('contacts bench');report['boots'].append(cold)
  assert 'ok initial_keys=8 keys=8 commits=0 sequence=1' in cold,cold
 assert len(set(report['cache_md5']))==1
 report['campaign_passed']=True
except Exception as error:report['error']=repr(error)
finally:
 try:
  rom('sb1-storage-app-restore','write-bin',65536,private/'sb1-private-app.bin')
  original=(private/'sb1-private-prefix.bin').read_bytes();tail=(private/'sb1-private-tail.bin').read_bytes()
  report['prefix_md5']=checksum('sb1-storage-restored-prefix',0,1048576)
  assert report['prefix_md5']==hashlib.md5(original).hexdigest()
  report['protected_except_owned_cache']={}
  for start,size in [(0,0xa000),(0xc000,0x4000)]:
   got=checksum('sb1-storage-protected-'+hex(start),0x3f0000+start,size)
   assert got==hashlib.md5(tail[start:start+size]).hexdigest()
   report['protected_except_owned_cache'][hex(0x3f0000+start)]=got
  run('sb1-storage-restored',[sys.executable,'-B',repo/'testing/device_names.py','SB1','--name','SB1','--output',q/'sb1-storage-restored.json'])
  before=json.loads((private/'sb1-before.json').read_text());after=json.loads((q/'sb1-storage-restored.json').read_text())
  assert before['sessions'][0]['responses']==after['sessions'][0]['responses']
  report['restored']=True
 except Exception as error:report['restoration_error']=repr(error)
 report['passed']=report.get('campaign_passed',False) and report.get('restored',False)
 (q/'v4-storage.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
sys.exit(0 if report['passed'] else 1)
