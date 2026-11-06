"""Feasibility probe only: real firmware instructions, stop at the first missing peripheral.

Requires `pip install unicorn==2.1.4`. This is NOT an N120 emulation backend.
Never returns made-up register values to force a firmware boot.
"""
import argparse
import hashlib
import json
import pathlib
import struct
import unicorn as uc
from unicorn import arm_const as arm

ap = argparse.ArgumentParser()
ap.add_argument('--internal', required=True, type=pathlib.Path)
ap.add_argument('--external', type=pathlib.Path)
ap.add_argument('--output', type=pathlib.Path)
args = ap.parse_args()
image = args.internal.read_bytes()
if not 8 <= len(image) <= 0x80000:
    ap.error('Internal image must contain vectors and fit 512 KiB')
sp, pc = struct.unpack_from('<II', image)
engine = uc.Uc(uc.UC_ARCH_ARM, uc.UC_MODE_THUMB | uc.UC_MODE_MCLASS)
engine.ctl_set_cpu_model(arm.UC_CPU_ARM_CORTEX_M7)
for base, size in [(0, 0x10000), (0x08000000, 0x80000), (0x20000000, 0x20000),
                   (0x24000000, 0x50000), (0x30000000, 0x8000), (0x38000000, 0x4000),
                   (0x90000000, 0x800000)]:
    engine.mem_map(base, size)
engine.mem_write(0x08000000, image)
engine.mem_write(0x90000000, b'\xff' * 0x800000)
if args.external:
    external = args.external.read_bytes()
    if len(external) > 0x800000:
        ap.error('External image exceeds 8 MiB')
    engine.mem_write(0x90000000, external)
engine.reg_write(arm.UC_ARM_REG_SP, sp)
engine.reg_write(arm.UC_ARM_REG_PC, pc)
result = {'engine': uc.__version__, 'cpu': 'Cortex-M7', 'instructions': 0,
          'internalSha256': hashlib.sha256(image).hexdigest(), 'localBoot': False}

def instruction(machine, address, size, data):
    result['instructions'] += 1

def missing(machine, access, address, size, value, data):
    result['missing'] = {'address': hex(address), 'access': access, 'size': size,
                         'value': hex(value), 'pc': hex(machine.reg_read(arm.UC_ARM_REG_PC))}
    return False

engine.hook_add(uc.UC_HOOK_CODE, instruction)
engine.hook_add(uc.UC_HOOK_MEM_UNMAPPED, missing)
try:
    engine.emu_start(pc, 0xFFFFFFFF, timeout=2000000, count=1000000)
except uc.UcError as error:
    result['stopReason'] = str(error)
result['pc'] = hex(engine.reg_read(arm.UC_ARM_REG_PC))
print(json.dumps(result, indent=2))
if args.output:
    args.output.write_text(json.dumps(result, indent=2))
