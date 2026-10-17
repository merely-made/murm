"""Fresh USB/status and ROM binding of existing immutable recovery bytes."""
from pathlib import Path
import hashlib,json,re,shutil,subprocess,sys,time
from serial.tools.list_ports import comports
p=Path(__file__).resolve().parent
repo=p.parents[2]
tool=repo/'validation/results/sennet-tucket-radios/artifacts/espflash.exe'
assert hashlib.sha256(tool.read_bytes()).hexdigest()=='0cc03364c70a86325236f18ad1aaed17eedf267d89312c0cdabe4964f5cb758e'
report={'passed':False,'boards':{}}
assert not (p/'preparation.json').exists()
def run(name,args):
 with (p/(name+'.log')).open('x') as log:
  result=subprocess.run(list(map(str,args)),stdout=log,stderr=subprocess.STDOUT,timeout=180)
 assert result.returncode==0,name
 return (p/(name+'.log')).read_text()
def md5(name,port,address,data):
 log=run(name,[tool,'checksum-md5','--port',port,'--chip','esp32s3','--before','usb-reset','--after','watchdog-reset','--baud','115200','--non-interactive','--skip-update-check',address,len(data)])
 time.sleep(6)
 expected=hashlib.md5(data).hexdigest()
 assert '0x'+expected in log,name+' differs from saved recovery bytes'
 return expected
try:
 for name,port,serial,source,prefix_name,tail_name in [
  ('SB1','COM6','44:1B:F6:6A:FB:28','tucket-contacts','sb1-private-prefix.bin','sb1-private-tail.bin'),
  ('SB2','COM7','44:1B:F6:6A:FA:64','firmware-memory','sb2-original-private-prefix.bin','sb2-original-private-tail.bin')]:
  assert any((v.device,v.vid,v.pid,v.serial_number)==(port,0x303a,0x1001,serial) for v in comports())
  run(name.lower()+'-before',[sys.executable,'-B',repo/'testing/device_names.py',name,'--name',name,'--output',p/(name.lower()+'-before.json')])
  original=repo/'validation/results'/source
  prefix=(original/prefix_name).read_bytes();tail=(original/tail_name).read_bytes()
  assert len(prefix)==1048576 and len(tail)==65536
  assert all(v==255 for v in tail[0x8000:0xa000]),'new contact allocation is not blank'
  record={'port':port,'usb_serial':serial,
   'prefix_md5':md5(name.lower()+'-prefix-before',port,0,prefix),
   'tail_md5':md5(name.lower()+'-tail-before',port,0x3f0000,tail)}
  for label,data in [('prefix',prefix),('tail',tail),('app',prefix[65536:])]:
   target=p/(name.lower()+'-private-'+label+'.bin');assert not target.exists();target.write_bytes(data)
  report['boards'][name]=record
 run('t114-before',[sys.executable,'-B',repo/'testing/protocol-capacity/board_probe.py','t114','--port','COM10','--output',p/'t114-before.json'])
 report['passed']=True
finally:
 (p/'preparation.json').write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps(report,indent=2))
