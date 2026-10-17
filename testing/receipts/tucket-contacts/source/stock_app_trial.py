"""Pinned application-only stock excursion with fresh rollback and ROM checks."""
from pathlib import Path
import hashlib
import json
import re
import subprocess
import sys
import time
from serial.tools.list_ports import comports

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
TOOL = REPO / 'validation/results/sennet-tucket-radios/artifacts/espflash.exe'
STOCK = TOOL.parent / 'heltec_v4_companion_radio_usb-v1.17.1-d929643-merged.bin'
BASE = ['--port', 'COM6', '--chip', 'esp32s3', '--before', 'usb-reset',
        '--after', 'watchdog-reset', '--baud', '115200', '--non-interactive', '--skip-update-check']
assert hashlib.sha256(TOOL.read_bytes()).hexdigest() == '0cc03364c70a86325236f18ad1aaed17eedf267d89312c0cdabe4964f5cb758e'
assert hashlib.sha256(STOCK.read_bytes()).hexdigest() == '67ef59c7687787255c23659e7525b81c1fc88bf679521f4183099a50580d0a3d'


def identity():
    assert any((p.device, p.vid, p.pid, p.serial_number) ==
               ('COM6', 0x303a, 0x1001, '44:1B:F6:6A:FB:28') for p in comports()), 'SB1 USB identity mismatch'


def run(name, args, timeout=360):
    path = ROOT / (name + '.log')
    with path.open('x') as log:
        result = subprocess.run(list(map(str, args)), stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
    assert result.returncode == 0, name
    return path.read_text()


def rom(name, cmd, *args):
    identity()
    try:
        return run(name, [TOOL, cmd, *BASE, *args])
    finally:
        time.sleep(6)


def md5(name, address, size):
    log = rom(name, 'checksum-md5', address, size)
    matches = re.findall(r'0x([0-9a-fA-F]{32})', log)
    assert len(matches) == 1, 'unrecognized ROM checksum output'
    return matches[0].lower()


def digest(data):
    return hashlib.md5(data).hexdigest()


def prepare():
    assert not (ROOT / 'rollback.json').exists()
    run('sb1-before', [sys.executable, REPO/'testing/device_names.py', 'SB1', '--name', 'SB1', '--output', ROOT/'sb1-before.json'])
    run('sb2-before', [sys.executable, REPO/'testing/device_names.py', 'SB2', '--name', 'SB2', '--output', ROOT/'sb2-before.json'])
    prefix = ROOT/'sb1-private-prefix.bin'
    tail = ROOT/'sb1-private-tail.bin'
    rom('sb1-backup-prefix', 'read-flash', '--block-size', 256, '--max-in-flight', 1, 0, 1048576, prefix)
    assert prefix.stat().st_size == 1048576
    assert md5('sb1-backup-prefix-md5', 0, 1048576) == digest(prefix.read_bytes())
    rom('sb1-backup-tail', 'read-flash', '--block-size', 256, '--max-in-flight', 1, 0x3f0000, 65536, tail)
    assert tail.stat().st_size == 65536
    assert md5('sb1-backup-tail-md5', 0x3f0000, 65536) == digest(tail.read_bytes())
    table = prefix.read_bytes()[0x8000:0x9000]
    # Only the known factory-app table is admitted. Never replace it with stock's OTA layout.
    import struct
    partitions = []
    for pos in range(0, len(table), 32):
        entry = table[pos:pos+32]
        if entry[:2] != b'\xaa\x50':
            break
        _, kind, subtype, offset, size, label, _ = struct.unpack('<HBBII16sI', entry)
        partitions.append(dict(kind=kind, subtype=subtype, offset=offset, size=size, label=label.rstrip(b'\0').decode()))
    assert [(p['offset'],p['size']) for p in partitions] == [(0x9000,0x6000),(0xf000,0x1000),(0x10000,0xfa0000)], partitions
    data = STOCK.read_bytes()[65536:]
    assert data[0] == 0xe9 and len(data) == 641728
    (ROOT/'stock-app.bin').write_bytes(data)
    (ROOT/'sb1-private-app.bin').write_bytes(prefix.read_bytes()[65536:])
    (ROOT/'sb1-private-data.bin').write_bytes(prefix.read_bytes()[0x9000:65536])
    report = dict(prefix_md5=digest(prefix.read_bytes()), tail_md5=digest(tail.read_bytes()),
                  data_md5=digest(prefix.read_bytes()[0x9000:65536]), partitions=partitions,
                  stock_app_sha256=hashlib.sha256(data).hexdigest(), write_range=[65536,65536+len(data)],
                  restore_app_range=[65536,1048576], protected_tail=[0x3f0000,0x400000])
    (ROOT/'rollback.json').write_text(json.dumps(report, indent=2)+'\n')
    print('Fresh prefix and protected tail match ROM checksums; application-only rollback ready.')


def install():
    report = json.loads((ROOT/'rollback.json').read_text())
    assert md5('sb1-install-precheck',0,1048576) == report['prefix_md5']
    rom('stock-app-install','write-bin',65536,ROOT/'stock-app.bin')
    print('Stock application installed; bootloader, partition table and protected tail were not written.')


def restore():
    report = json.loads((ROOT/'rollback.json').read_text())
    restored = dict(passed=False)
    try:
        rom('sb1-app-restore','write-bin',65536,ROOT/'sb1-private-app.bin')
        restored['data_before_restore_md5'] = md5('sb1-data-before-restore',0x9000,0x7000)
        if restored['data_before_restore_md5'] != report['data_md5']:
            rom('sb1-data-restore','write-bin',0x9000,ROOT/'sb1-private-data.bin')
            restored['data_restored'] = True
        restored['prefix_md5'] = md5('sb1-restored-prefix-md5',0,1048576)
        restored['tail_md5'] = md5('sb1-restored-tail-md5',0x3f0000,65536)
        assert restored['prefix_md5'] == report['prefix_md5']
        assert restored['tail_md5'] == report['tail_md5']
        run('sb1-restored', [sys.executable, REPO/'testing/device_names.py', 'SB1', '--name', 'SB1', '--output', ROOT/'sb1-restored.json'])
        before = json.loads((ROOT/'sb1-before.json').read_text())
        after = json.loads((ROOT/'sb1-restored.json').read_text())
        assert before['sessions'][0]['responses'] == after['sessions'][0]['responses']
        restored['passed'] = True
    except Exception as error:
        restored['error'] = repr(error)
        raise
    finally:
        with (ROOT/'sb1-restoration.json').open('x') as output:
            json.dump(restored, output, indent=2)
        print(json.dumps(restored))


{'prepare':prepare,'install':install,'restore':restore}[sys.argv[1]]()
