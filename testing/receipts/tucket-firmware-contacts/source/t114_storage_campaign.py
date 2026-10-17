"""T114 real-flash storage only; no standalone Tucket service/RF claim."""
from pathlib import Path
import hashlib,json,re,subprocess,sys,time
from zipfile import ZipFile
import serial
from serial.tools.list_ports import comports
base=Path(__file__).resolve().parent;p=base/'corrected';repo=base.parents[2]
tool=Path(r'C:\Users\mark_\AppData\Local\hermes\hermes-agent\venv\Scripts\adafruit-nrfutil.exe')
assert hashlib.sha256(tool.read_bytes()).hexdigest()=='458cd93f99c0ac4aa85c5e9a8a5cd9ffde4144c211d499a4ab5cf2cc9704fd9d'
old=repo/'validation/results/firmware-memory'
packages={'bench':p/'t114-contacts.zip','restore':old/'t114-restore.zip','handoff':old/'t114-handoff.zip'}
assert hashlib.sha256(packages['restore'].read_bytes()).hexdigest()=='4c56670f0cd51387b26802b29fe160439ce173a8c66e0538d6e97a3113d6605d'
assert hashlib.sha256(packages['handoff'].read_bytes()).hexdigest()=='7ef7e6c2d50e8aa5e5536cdcda124cb2dda9636dcb40fa6dd9efb32feae41740'
report={'passed':False,'source':(p/'source-revision.txt').read_text().strip(),'boots':[],'packages':{},'scope':'application-only DFU and internal public-cache readback; no protected-flash dump'}
assert not (p/'t114-storage.json').exists()
for name,path in packages.items():
 with ZipFile(path) as z:
  m=json.loads(z.read('manifest.json'))['manifest'];assert set(m)=={'application','dfu_version'}
  data=z.read(m['application']['bin_file']);assert len(data)%4==0 and 0x26000+(len(data)+4095)//4096*4096<=0xe4000
  report['packages'][name]={'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'bytes':len(data),'erase_end':hex(0x26000+(len(data)+4095)//4096*4096)}
def is_port(port,vid,pid,serial):return any((v.device,v.vid,v.pid,v.serial_number)==(port,vid,pid,serial) for v in comports())
def app():return is_port('COM10',0x1915,0x521f,'TULLE-T114-01')
def loader():return is_port('COM4',0x239a,0x0071,'AA6D3CD5E3BD5626')
def wait(check):
 until=time.monotonic()+20
 while time.monotonic()<until:
  if check():return
  time.sleep(.25)
 raise TimeoutError('identified T114 USB endpoint absent')
def query(command):
 wait(app);link=serial.Serial(port=None,baudrate=115200,timeout=.1,write_timeout=2)
 link.port,link.rts,link.dtr='COM10',False,False
 with link:
  time.sleep(.3);link.dtr=True;time.sleep(.3);link.read(4096)
  link.write(command.encode()+b'\n');link.flush();response=bytearray();until=time.monotonic()+1.5
  while time.monotonic()<until:response.extend(link.read(4096))
 return response.decode(errors='replace')
def enter():
 wait(lambda:app() or loader())
 if app():
  link=serial.Serial(port=None,baudrate=115200,timeout=.1,write_timeout=2)
  link.port,link.rts,link.dtr='COM10',False,True
  with link:
   time.sleep(.3);link.read(4096);link.write(b'bootloader\n');link.flush()
  wait(loader)
def run(name,args,timeout=240):
 with (p/(name+'.log')).open('x') as log:r=subprocess.run(list(map(str,args)),stdout=log,stderr=subprocess.STDOUT,timeout=timeout)
 assert r.returncode==0,name
 return (p/(name+'.log')).read_text()
def flash(name,kind,expect_app=True):
 assert loader()
 log=run(name,[tool,'dfu','serial','--package',packages[kind],'--port','COM4','--baudrate','115200','--singlebank'])
 assert 'Device programmed.' in log
 if expect_app:wait(app)
 else:time.sleep(12);wait(loader)
def staged(name,kind):enter();flash(name+'-bridge','handoff',False);flash(name+'-app',kind)
try:
 staged('t114-storage-install','bench')
 first=query('contacts bench');report['boots'].append(first)
 assert 'ok initial_keys=0 keys=8 commits=2 sequence=1' in first,first
 run('t114-storage-memory',[sys.executable,'-B',repo/'testing/protocol-capacity/board_probe.py','t114','--port','COM10','--rounds',16,'--min-stack-bytes','16384','--min-free-heap-percent','25','--output',p/'t114-storage-memory.json'],360)
 staged('t114-storage-cold','bench')
 cold=query('contacts bench');report['boots'].append(cold)
 assert 'ok initial_keys=8 keys=8 commits=0 sequence=1' in cold,cold
 report['cold_flash_counts']=query('flashcounts')
 report['campaign_passed']=True
except Exception as error:report['error']=repr(error)
finally:
 try:
  staged('t114-storage-restore','restore')
  run('t114-storage-restored',[sys.executable,'-B',repo/'testing/protocol-capacity/board_probe.py','t114','--port','COM10','--output',p/'t114-storage-restored.json'])
  before=json.loads((p/'t114-before.json').read_text());after=json.loads((p/'t114-storage-restored.json').read_text())
  norm=lambda s:re.sub(r'reset=\w+ crash=\d+','reset=HANDOFF crash=HANDOFF',s)
  assert all(norm(a['response'])==norm(b['response']) for a,b in zip(before['responses'][:4],after['responses'][:4]))
  assert all(s in after['attached'] for s in ['identity=loaded slot=A seq=94','node=599997c8'])
  report['restored']=True
 except Exception as error:report['restoration_error']=repr(error)
 report['passed']=report.get('campaign_passed',False) and report.get('restored',False)
 (p/'t114-storage.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
sys.exit(0 if report['passed'] else 1)
