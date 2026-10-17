from pathlib import Path
import subprocess,json,sys,time,hashlib
from serial.tools.list_ports import comports
p=Path('validation/results/firmware-memory'); tool=Path('validation/results/sennet-tucket-radios/artifacts/espflash.exe')
assert hashlib.sha256(tool.read_bytes()).hexdigest()=='0cc03364c70a86325236f18ad1aaed17eedf267d89312c0cdabe4964f5cb758e'
report={'passed':False,'actions':[],'firmware_source':'a7107a7266f'}; output=p/'v4-pinned-physical.json'; assert not output.exists()
def run(name,args,timeout=300):
 with (p/(name+'.log')).open('x') as log:
  r=subprocess.run(list(map(str,args)),stdout=log,stderr=subprocess.STDOUT,timeout=timeout)
 report['actions'].append(dict(name=name,command=list(map(str,args)),exit_code=r.returncode)); assert r.returncode==0,name
base=[tool,'--port','COM7','--chip','esp32s3','--before','usb-reset','--after','watchdog-reset','--baud','115200','--non-interactive','--skip-update-check']
def identity():
 assert any((x.device,x.vid,x.pid,x.serial_number)==('COM7',0x303a,0x1001,'44:1B:F6:6A:FA:64') for x in comports()),'SB2 USB identity mismatch'
def flash(name,image):
 identity(); run(name,[base[0],'write-bin',*base[1:],65536,p/image]); time.sleep(6)
def md5(name,address,size,original):
 identity(); run(name,[base[0],'checksum-md5',*base[1:],address,size]); time.sleep(6)
 expected=hashlib.md5(original).hexdigest(); observed=(p/(name+'.log')).read_text()
 assert '0x'+expected in observed,'ROM checksum differs from fresh backup'
 return expected
try:
 flash('v4-pinned-install','v4-pinned.bin')
 run('v4-pinned-campaign',[sys.executable,'testing/protocol-capacity/board_probe.py','v4','--port','COM7','--rounds',16,'--min-stack-bytes',16384,'--min-free-heap-percent',25,'--output',p/'v4-pinned-campaign.json'])
 campaign=json.loads((p/'v4-pinned-campaign.json').read_text()); assert all('build=a7107a7266f' in v['response'] for v in campaign['responses'] if v['command']=='status')
 report['campaign_passed']=True
except Exception as error: report['error']=repr(error)
finally:
 try:
  flash('v4-pinned-restore','sb2-original-private-app.bin')
  report['prefix_rom_md5']=md5('v4-pinned-restored-prefix-md5',0,1048576,(p/'sb2-original-private-prefix.bin').read_bytes())
  try:
   identity(); run('v4-pinned-restored-tail',[base[0],'read-flash',*base[1:],'--block-size',512,'--max-in-flight',1,0x3f0000,65536,p/'sb2-pinned-restored-private-tail.bin']); time.sleep(6)
   assert (p/'sb2-pinned-restored-private-tail.bin').read_bytes()==(p/'sb2-original-private-tail.bin').read_bytes()
   report['protected_tail_exact']=True
  except Exception as error:
   report['tail_read_error']=repr(error)
   report['protected_tail_rom_md5']=md5('v4-pinned-restored-tail-md5',0x3f0000,65536,(p/'sb2-original-private-tail.bin').read_bytes())
  run('v4-pinned-readonly',[sys.executable,'testing/device_names.py','SB2','--name','SB2','--output',p/'sb2-pinned-restored.json'])
  before=json.loads((p/'sb2-before.json').read_text()); after=json.loads((p/'sb2-pinned-restored.json').read_text())
  assert before['sessions'][0]['responses']==after['sessions'][0]['responses'],'restored name/status/profile differs'
  report['original_status_profile_exact']=True
  report['passed']=report.get('campaign_passed',False) and ('prefix_rom_md5' in report) and (report.get('protected_tail_exact',False) or 'protected_tail_rom_md5' in report)
 except Exception as error: report['restoration_error']=repr(error)
 output.write_text(json.dumps(report,indent=2)+'\n'); print(json.dumps(report,indent=2))
sys.exit(0 if report['passed'] else 1)
