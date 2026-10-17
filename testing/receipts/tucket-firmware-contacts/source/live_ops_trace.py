"""Exact-identity app-only V4 actions and preservation checks for attended trial."""
from pathlib import Path
import hashlib,json,re,subprocess,sys,time,zlib
from serial.tools.list_ports import comports
base=Path(__file__).resolve().parent;p=base/'setup-trace';private=base/'corrected';repo=base.parents[2]
trial=next((s[6:] for s in sys.argv if s.startswith('trial=')),None)
assert trial is None or re.fullmatch(r'[a-z0-9-]+',trial)
q=p/trial if trial else p
tool=repo/'validation/results/sennet-tucket-radios/artifacts/espflash.exe'
assert hashlib.sha256(tool.read_bytes()).hexdigest()=='0cc03364c70a86325236f18ad1aaed17eedf267d89312c0cdabe4964f5cb758e'
who=sys.argv[2];port,usb={'SB1':('COM6','44:1B:F6:6A:FB:28'),'SB2':('COM7','44:1B:F6:6A:FA:64')}[who]
def identity():assert any((v.device,v.vid,v.pid,v.serial_number)==(port,0x303a,0x1001,usb) for v in comports())
def run(name,args):
 assert re.fullmatch(r'[a-z0-9-]+',name)
 with (q/(name+'.log')).open('x') as log:r=subprocess.run(list(map(str,args)),stdout=log,stderr=subprocess.STDOUT,timeout=300)
 assert r.returncode==0,name
 return (q/(name+'.log')).read_text()
def rom(name,mode,*args):
 identity()
 try:return run(name,[tool,mode,'--port',port,'--chip','esp32s3','--before','usb-reset','--after','watchdog-reset','--baud','115200','--non-interactive','--skip-update-check',*args])
 finally:time.sleep(6)
def md5(name,origin,size):
 text=rom(name,'checksum-md5',origin,size)
 values=re.findall(r'0x([0-9a-fA-F]{32})',text);assert len(values)==1
 return values[0].lower()
def flash(name,path):
 data=path.read_bytes();assert data[0]==0xe9 and 0<len(data)<=0xf0000
 pins={**json.loads((private/'images.json').read_text()),**json.loads((p/'images.json').read_text())}
 if path.name in pins:assert hashlib.sha256(data).hexdigest()==pins[path.name]['sha256']
 else:
  stock=repo/'validation/results/sennet-tucket-radios/artifacts/heltec_v4_companion_radio_usb-v1.17.1-d929643-merged.bin'
  assert hashlib.sha256(stock.read_bytes()).hexdigest()=='67ef59c7687787255c23659e7525b81c1fc88bf679521f4183099a50580d0a3d'
  assert data==stock.read_bytes()[65536:]
 rom(name,'write-bin',65536,path)
def records(tail,offset):
 result=[]
 for start in [offset,offset+4096]:
  page=tail[start:start+4096]
  if all(v==255 for v in page):continue
  assert page[:4]==b'RHS0' and int.from_bytes(page[4:6],'little')==1
  length=int.from_bytes(page[6:8],'little');body=page[16:16+length]
  assert zlib.crc32(page[:12]+body)==int.from_bytes(page[12:16],'little')
  result.append((int.from_bytes(page[8:12],'little'),body))
 return [body for _,body in sorted(result)]
def restore():
 name=who.lower();result={'passed':False}
 try:
  flash(name+'-live-restore',private/(name+'-private-app.bin'))
  prefix=(private/(name+'-private-prefix.bin')).read_bytes();before=(private/(name+'-private-tail.bin')).read_bytes()
  result['prefix_md5']=md5(name+'-live-prefix-restored',0,1048576)
  assert result['prefix_md5']==hashlib.md5(prefix).hexdigest()
  after_path=q/(name+'-live-private-tail.bin')
  rom(name+'-live-tail-read','read-flash','--block-size',256,'--max-in-flight',1,0x3f0000,65536,after_path)
  after=after_path.read_bytes();assert len(after)==65536
  result['tail_md5']=md5(name+'-live-tail-md5',0x3f0000,65536)
  assert hashlib.md5(after).hexdigest()==result['tail_md5']
  spans=[(0,0x2000),(0x4000,0x8000),(0xc000,0x10000)] if who=='SB2' else [(0,0xa000),(0xc000,0x10000)]
  assert all(before[a:b]==after[a:b] for a,b in spans)
  result['immutable_tail_ranges_exact']=[[hex(0x3f0000+a),hex(0x3f0000+b)] for a,b in spans]
  if who=='SB2':
   old=records(before,0x8000);new=records(after,0x8000)
   assert all(body[:4]==b'SNP1' for body in old+new)
   old_next=int.from_bytes(old[-1][4:8],'little') if old else 1
   new_next=int.from_bytes(new[-1][4:8],'little');assert new_next>=old_next
   old_a=records(before,0x2000);new_a=records(after,0x2000)
   assert all(body[:4]==b'RHR0' for body in old_a+new_a)
   old_through=int.from_bytes(old_a[-1][7:12],'big') if old_a else 0
   new_through=int.from_bytes(new_a[-1][7:12],'big');assert new_through>=old_through
   result['packet_ceiling']={'before':old_next,'after':new_next}
   result['announce_ceiling']={'before':old_through,'after':new_through}
  run(name+'-live-readonly',[sys.executable,'-B',repo/'testing/device_names.py',who,'--name',who,'--output',q/(name+'-live-readonly.json')])
  first=json.loads((private/(name+'-before.json')).read_text());last=json.loads((q/(name+'-live-readonly.json')).read_text())
  assert first['sessions'][0]['responses']==last['sessions'][0]['responses']
  result['passed']=True
 except Exception as error:result['error']=repr(error);raise
 finally:
  (q/(name+'-live-restoration.json')).write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
mode=sys.argv[1]
if mode=='flash':flash(sys.argv[4],p/sys.argv[3])
elif mode=='reset':print(md5(sys.argv[3],0x3fa000,8192))
elif mode=='restore':restore()
else:raise ValueError(mode)
