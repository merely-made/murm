"""Attended T114 application-only actions for the live Tucket receipt.

Every invocation has a fresh output directory; failed output is retained.
No settings/control/reservation page or private flash dump is written.
"""
from pathlib import Path
from zipfile import ZipFile
import argparse
import hashlib
import json
import re
import subprocess
import time
import serial
from serial.tools.list_ports import comports

REPO = Path(__file__).resolve().parents[1]
BASE = REPO / 'validation/results/tucket-t114'
TOOL = Path(r'C:\Users\mark_\AppData\Local\hermes\hermes-agent\venv\Scripts\adafruit-nrfutil.exe')
APP = ('COM10', 0x1915, 0x521f, 'TULLE-T114-01')
LOADER = ('COM4', 0x239a, 0x0071, 'AA6D3CD5E3BD5626')


def present(identity):
    return any((p.device, p.vid, p.pid, p.serial_number) == identity for p in comports())


def wait(check):
    until = time.monotonic() + 25
    while time.monotonic() < until:
        if check():
            return
        time.sleep(.25)
    raise TimeoutError('identified T114 endpoint absent')


def query(command, resetting=False):
    wait(lambda: present(APP))
    link = serial.Serial(port=None, baudrate=115200, timeout=.1, write_timeout=2)
    link.port, link.rts, link.dtr = APP[0], False, False
    with link:
        time.sleep(.3)
        link.dtr = True
        time.sleep(.3)
        link.read(4096)
        link.write(command.encode() + b'\n')
        link.flush()
        if resetting:
            return 'reset requested'
        result = bytearray()
        until = time.monotonic() + 1.6
        while time.monotonic() < until:
            result.extend(link.read(4096))
            assert len(result) <= 65536
    return result.decode(errors='replace')


def package(kind):
    old = REPO / 'validation/results/firmware-memory'
    if kind in ('restore', 'bridge'):
        path = old / ('t114-restore.zip' if kind == 'restore' else 't114-handoff.zip')
        expected = {'restore': '4c56670f0cd51387b26802b29fe160439ce173a8c66e0538d6e97a3113d6605d',
                    'bridge': '7ef7e6c2d50e8aa5e5536cdcda124cb2dda9636dcb40fa6dd9efb32feae41740'}[kind]
    else:
        path = BASE / 'images' / f't114-{kind}.zip'
        expected = json.loads((BASE / 'images/images.json').read_text())[path.name]['sha256']
    assert hashlib.sha256(path.read_bytes()).hexdigest() == expected
    with ZipFile(path) as archive:
        manifest = json.loads(archive.read('manifest.json'))['manifest']
        assert set(manifest) == {'application', 'dfu_version'}
        data = archive.read(manifest['application']['bin_file'])
        assert len(data) % 4 == 0
        assert 0x26000 + (len(data) + 4095) // 4096 * 4096 <= 0xe4000
    return path


def run(out, name, args):
    with (out / (name + '.log')).open('x') as log:
        completed = subprocess.run(list(map(str, args)), stdout=log, stderr=subprocess.STDOUT, timeout=300)
    assert completed.returncode == 0, name
    return (out / (name + '.log')).read_text()


def install(out, kind):
    # Verify both packages and the programmer before entering the loader.
    paths = [package('bridge'), package(kind)]
    assert hashlib.sha256(TOOL.read_bytes()).hexdigest() == '458cd93f99c0ac4aa85c5e9a8a5cd9ffde4144c211d499a4ab5cf2cc9704fd9d'
    wait(lambda: present(APP) or present(LOADER))
    if present(APP):
        query('bootloader', resetting=True)
    wait(lambda: present(LOADER))
    for name, path in zip(('bridge', kind), paths):
        assert present(LOADER)
        text = run(out, name, [TOOL, 'dfu', 'serial', '--package', path, '--port', LOADER[0], '--baudrate', '115200', '--singlebank'])
        assert 'Device programmed.' in text
        if name == 'bridge':
            time.sleep(12)
            wait(lambda: present(LOADER))
    wait(lambda: present(APP))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['live', 'fixture-handoff', 'restore', 'reset', 'pressure'])
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    args.output.mkdir(exist_ok=False)
    receipt = {'passed': False, 'action': args.action, 'usb': APP}
    try:
        if args.action in ('live', 'fixture-handoff', 'restore'):
            install(args.output, args.action)
        if args.action == 'fixture-handoff':
            receipt['handoff'] = query('contacts bench')
            receipt['flash'] = query('flashcounts')
            assert 'ok initial_keys=0 keys=0 commits=0 sequence=0' in receipt['handoff']
            assert 'erases=2 writes=0' in receipt['flash']
        if args.action in ('restore', 'pressure'):
            output = args.output / 'board.json'
            command = [__import__('sys').executable, '-B', REPO / 'testing/protocol-capacity/board_probe.py', 't114', '--port', APP[0], '--output', output]
            if args.action == 'pressure':
                command += ['--rounds', '16', '--min-stack-bytes', '16384', '--min-free-heap-percent', '25']
            run(args.output, 'probe', command)
            after = json.loads(output.read_text())
            assert after['passed']
            if args.action == 'restore':
                before = json.loads((BASE / 't114-before.json').read_text())
                norm = lambda s: re.sub(r'reset=\w+ crash=\d+', 'reset=HANDOFF crash=HANDOFF', s)
                assert all(norm(a['response']) == norm(b['response']) for a, b in zip(before['responses'][:4], after['responses'][:4]))
                assert all(s in after['attached'] for s in ['identity=loaded slot=A seq=94', 'node=599997c8'])
                receipt['scope'] = 'live original identity/profile/application; no protected-flash dump'
        if args.action == 'reset':
            query('reboot', resetting=True)
            time.sleep(3)
            wait(lambda: present(APP))
        receipt['passed'] = True
    except Exception as error:
        receipt['error'] = repr(error)
        raise
    finally:
        (args.output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')


if __name__ == '__main__':
    main()
