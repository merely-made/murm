"""Controlled public contact and send oracle using the untouched stock CLI.

No implementation/schema source is read. Contact bytes are observed from the
supported public QR command; only observed public identity leaves are replaced.
The CLI must import and re-export that exact identity before a tagged RF send.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
from setup_sennet_stock import identified, public_diagnostics
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"crates/sennet/capture"))
from compare_config import parse_message

def varint(value):
    result=bytearray()
    while value>=128:
        result.append((value&127)|128); value>>=7
    result.append(value)
    return bytes(result)

def url_bytes(result):
    urls=re.findall(r"https://meshtastic\.org/v/#[A-Za-z0-9_=-]+",result.stdout)
    if result.returncode or len(set(urls))!=1: raise RuntimeError("unique public contact URL missing")
    url=urls[0]
    data=base64.urlsafe_b64decode(url.split('#')[1]+'===')
    if len(data)>2048: raise RuntimeError("public contact exceeds bound")
    return url,data

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('action',choices=('contact','send'))
    p.add_argument('--port',required=True); p.add_argument('--usb-serial',required=True)
    p.add_argument('--station',required=True,type=Path); p.add_argument('--setup',required=True,type=Path)
    p.add_argument('--output',required=True,type=Path); p.add_argument('--text')
    p.add_argument('--require-ack',action='store_true')
    a=p.parse_args()
    if a.output.exists(): p.error('preserve earlier receipt')
    station=json.loads(a.station.read_text(encoding='utf-8-sig'))
    setup=json.loads(a.setup.read_text(encoding='utf-8-sig'))
    if station['authority']!='djinn' or not station['realm'].startswith('bench-') or setup['result']!='passed': p.error('require qualified bench identities')
    def cli(*args):
        identified(a.port,a.usb_serial)
        return subprocess.run([sys.executable,'-B','-m','meshtastic','--port',a.port,'--wait-to-disconnect','5',*args],capture_output=True,text=True,encoding='utf-8',errors='replace',env={**os.environ,'PYTHONIOENCODING':'utf-8'},timeout=60)
    report={'result':'not_completed','action':a.action,'node_number':station['node_number'],'peer_node':setup['node_number'],'private_material_included':False}
    try:
        if a.action=='contact':
            stock_id=('!%08x'%setup['node_number']).encode()
            own_id=('!%08x'%station['node_number']).encode()
            stock_key=bytes.fromhex(setup['pki_public_key']); own_key=bytes.fromhex(station['pki_public_key'])
            if len(stock_key)!=32 or len(own_key)!=32 or stock_key==own_key: raise RuntimeError('distinct full public keys required')
            stock_url,data=url_bytes(cli('--contact-qr',stock_id.decode(),'--contact-verified'))
            report['stock_contact_url']=stock_url
            # The captured public contact has both a numeric outer node and a
            # nested textual ID. Both must identify the new scoped station.
            stock_mac=bytes.fromhex(a.usb_serial.replace(':',''))
            user_fields=parse_message(next(v for n,w,v in parse_message(data) if n==2 and w==2))
            stock_long=next(v for n,w,v in user_fields if n==2 and w==2)
            stock_short=next(v for n,w,v in user_fields if n==3 and w==2)
            counts={'text_id':0,'public_key':0,'numeric_node':0,'hardware_mac_omitted':0,'long_name':0,'short_name':0}
            def replace(data,depth=0):
                fields=parse_message(data)
                if fields is None or depth>8: return data
                result=bytearray()
                for number,wire,value in fields:
                    if wire==2 and value==stock_mac:
                        counts['hardware_mac_omitted']+=1
                        continue
                    result.extend(varint((number<<3)|wire))
                    if wire==2:
                        if value==stock_id: value=own_id; counts['text_id']+=1
                        elif value==stock_key: value=own_key; counts['public_key']+=1
                        elif value==stock_long: value=b'Signalman Sennet bench'; counts['long_name']+=1
                        elif value==stock_short: value=own_id[-4:]; counts['short_name']+=1
                        else: value=replace(value,depth+1)
                        result.extend(varint(len(value))); result.extend(value)
                    elif wire==0:
                        if value==setup['node_number']: value=station['node_number']; counts['numeric_node']+=1
                        result.extend(varint(value))
                    else: result.extend(value)
                return bytes(result)
            replaced=replace(data)
            if any(v!=1 for v in counts.values()): raise RuntimeError('observed contact public identity leaves differ from the controlled capture')
            own_url='https://meshtastic.org/v/#'+base64.urlsafe_b64encode(replaced).decode().rstrip('=')
            report['station_contact_url']=own_url
            report['replaced_exact_public_leaves']=counts
            imported=cli('--add-contact',own_url)
            report['import']={'exit':imported.returncode,'public_diagnostics':public_diagnostics(imported)}
            if imported.returncode: raise RuntimeError('supported contact import failed')
            time.sleep(2)
            from meshtastic.serial_interface import SerialInterface
            interface=SerialInterface(devPath=a.port)
            try:
                interface.waitForConfig()
                observed=interface.nodesByNum.get(station['node_number'],{}).get('user',{})
                observed_key=observed.get('publicKey',b'')
                if isinstance(observed_key,str): observed_key=base64.b64decode(observed_key)
                matches=observed.get('id')==own_id.decode() and bytes(observed_key)==own_key
                report['selected_sdk_readback']={'present':bool(observed),'id_match':observed.get('id')==own_id.decode(),'public_key_match':bytes(observed_key)==own_key}
            finally: interface.close()
            time.sleep(2)
            roundtrip=cli('--contact-qr',own_id.decode(),'--contact-verified')
            report['qr_readback']={'exit':roundtrip.returncode,'public_diagnostics':public_diagnostics(roundtrip)}
            try:
                round_url,round_data=url_bytes(roundtrip)
                if own_id not in round_data or own_key not in round_data: raise RuntimeError('QR readback public identity mismatch')
                report['readback_contact_url']=round_url
                matches=True
            except RuntimeError:
                if not matches: raise RuntimeError('neither SDK nor QR readback confirms the imported public identity') from None
            report['result']='public_contact_import_read_back'
        else:
            if not a.text or not a.text.startswith('sennet-pki-') or len(a.text)>100: raise RuntimeError('explicit bounded bench tag required')
            command=['--dest','!%08x'%station['node_number'],'--sendtext',a.text,'--timeout','25']
            if a.require_ack: command+=['--ack']
            sent=cli(*command)
            report.update(cli_exit=sent.returncode,public_diagnostics=public_diagnostics(sent),text=a.text)
            if sent.returncode: raise RuntimeError('stock send command failed')
            matched=bool(re.search(r'Received an ACK\.',sent.stdout))
            report['matched_client_protocol_ack']=matched
            report['authenticated_ack_proof']=False
            if a.require_ack and not matched: raise RuntimeError('stock client did not observe a protocol ACK')
            report['result']='stock_client_protocol_ack_observed' if matched else 'stock_send_command_completed_not_delivery_proof'
    except Exception as error:
        report['result']='failed'; report['error_type']=type(error).__name__; raise
    finally: a.output.write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')

if __name__=='__main__': main()
