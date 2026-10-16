"""Read ELF sections, target ABI layout symbols and linker stack bounds.

No target is executed. Load-segment sizes are not a merged flash-package size.
"""
import argparse
import hashlib
import json
import struct
from pathlib import Path


def measure(path):
    data = path.read_bytes()
    if data[:4] != b'\x7fELF' or data[5] != 1 or data[4] != 1:
        raise ValueError('require little-endian ELF32')
    header = struct.unpack_from('<16sHHIIIIIHHHHHH', data)
    _, _, machine, _, entry, phoff, shoff, _, _, phsize, phnum, shsize, shnum, names = header
    sections = [struct.unpack_from('<IIIIIIIIII', data, shoff+i*shsize) for i in range(shnum)]
    strings = data[sections[names][4]:sections[names][4]+sections[names][5]]
    def name(table, offset):
        return table[offset:table.index(0,offset)].decode('utf-8')
    symbols = {}
    layouts = {}
    for section in sections:
        if section[1] != 2:
            continue
        table = sections[section[6]]
        labels = data[table[4]:table[4]+table[5]]
        for offset in range(section[4],section[4]+section[5],section[9]):
            label,value,size,_,_,index = struct.unpack_from('<IIIBBH',data,offset)
            label = name(labels,label)
            if label in ('_stack_start_cpu0','_stack_end_cpu0','_stack_start','__sheap','__euninit'):
                symbols[label] = value
            if label.startswith('PROTOCOL_LAYOUT_V') and 0 < index < len(sections):
                owner = sections[index]
                position = owner[4] + value - owner[3]
                layouts[label] = list(struct.unpack_from('<'+'I'*(size//4),data,position))
    allocated = [dict(name=name(strings,s[0]),address=s[3],bytes=s[5],type=s[1])
                 for s in sections if s[2]&2 and s[5]]
    loads = []
    for i in range(phnum):
        kind,_,vaddr,paddr,filesz,memsz,flags,_ = struct.unpack_from('<IIIIIIII',data,phoff+i*phsize)
        if kind == 1:
            loads.append(dict(virtual_address=vaddr,physical_address=paddr,file_bytes=filesz,memory_bytes=memsz,flags=flags))
    result = dict(path=str(path.resolve()),sha256=hashlib.sha256(data).hexdigest(),elf_bytes=len(data),
                  machine=machine,entry=entry,allocated_sections=allocated,load_segments=loads,
                  target_layouts=layouts,linker_symbols=symbols)
    if machine == 40:
        ram = [s for s in allocated if 0x20006000 <= s['address'] < 0x20040000]
        end = max(s['address']+s['bytes'] for s in ram)
        result['application_ram'] = dict(capacity=232*1024,allocated_sections=sum(s['bytes'] for s in ram),highest_allocated_end=end,unallocated_above_sections=0x20040000-end,scope='linker gap, not measured stack high-water')
        flash = [s for s in loads if s['physical_address'] < 0x100000 and s['file_bytes']]
        assert all(0x26000 <= s['physical_address'] and
                   s['physical_address']+s['file_bytes'] <= 0xe6000 for s in flash)
        end = max(s['physical_address']+s['file_bytes'] for s in flash)
        assert end <= 0xe6000
        result['application_flash'] = dict(capacity=768*1024,load_end=end,span_from_origin=end-0x26000)
    elif machine == 94 and '_stack_start_cpu0' in symbols:
        result['stack_region_bytes'] = symbols['_stack_start_cpu0']-symbols['_stack_end_cpu0']
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('elf', type=Path)
    parser.add_argument('--output',type=Path,required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error('preserve previous measurements')
    result = measure(args.elf)
    args.output.write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(dict(sha256=result['sha256'],layouts=result['target_layouts'],ram=result.get('application_ram'),stack_region=result.get('stack_region_bytes'))))
