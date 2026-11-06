"""Real Renode GDB remote protocol and reset-vector selection checks."""
import argparse,json,pathlib,socket,struct
from integration import Engine,free_port,ROOT
from integration import arguments
a=arguments()
results=[]
def check(name,value):
 print(('PASS ' if value else 'FAIL ')+name,flush=True);results.append(dict(name=name,passed=bool(value)));assert value,name
e=Engine(a)
try:
 e.machine(0x4e4101);e.run(.1)
 port=free_port();e.cmd('lcd StartDebug %d'%port)
 with socket.create_connection(('localhost',port),timeout=5) as g:
  def receive():
   while True:
    byte=g.recv(1)
    if not byte:raise EOFError('GDB disconnected before packet')
    if byte==b'$':break
   data=b''
   while True:
    c=g.recv(1)
    if c==b'#':break
    if not c:raise EOFError()
    data+=c
   checksum=b''
   while len(checksum)<2:
    byte=g.recv(2-len(checksum))
    if not byte:raise EOFError('GDB disconnected during checksum')
    checksum+=byte
   assert int(checksum,16)==sum(data)%256,(data,checksum)
   g.sendall(b'+')
   decoded=bytearray();i=0
   while i<len(data):
    if data[i]==42:decoded.extend(bytes([decoded[-1]])*(data[i+1]-29));i+=2
    elif data[i]==125:decoded.append(data[i+1]^32);i+=2
    else:decoded.append(data[i]);i+=1
   return decoded.decode()
  initial=receive();print('Initial stop',initial,flush=True)
  def rsp(text):
   data=text.encode();g.sendall(b'$'+data+b'#'+('%02x'%(sum(data)%256)).encode());return receive()
  supported=rsp('qSupported');print('GDB features',supported,flush=True);check('GDB qSupported handshake','qXfer' in supported or 'PacketSize' in supported)
  check('GDB halt status',rsp('?').startswith(('S','T')))
  registers=rsp('g');print('Registers',registers,flush=True);check('GDB ARM register read',len(registers)>=16*8)
  memory=rsp('m8000000,8');check('GDB reads actual firmware vectors',bytes.fromhex(memory)==a.internal.read_bytes()[:8])
  check('GDB memory write',rsp('M20000100,4:12345678')=='OK' and rsp('m20000100,4')=='12345678')
  pc=int.from_bytes(bytes.fromhex(registers[15*8:16*8]),'little')
  check('GDB breakpoint insertion',rsp('Z0,%x,2'%pc)=='OK')
  check('GDB breakpoint removal',rsp('z0,%x,2'%pc)=='OK')
  e.cmd('start');check('GDB instruction step',rsp('s').startswith(('S','T')))
  rsp('D')
 e.cmd('lcd StopDebug');check('GDB server disabled',e.snapshot()['gdbPort']==0)
 # A clearly synthetic two-instruction fixture tests infrastructure only.
 # It is never presented as an STM32 system bootloader or DFU ROM.
 e.cmd('pause');e.write(0x1ff00000,0x20010000);e.write(0x1ff00004,0x1ff00009);e.write(0x1ff00008,0xE7FEBF00)
 e.cmd('lcd ConfigureBoot true');e.cmd('machine Reset');e.run(.00001)
 s=e.snapshot();check('BOOT0 samples system vector on reset',0x1ff00008<=s['pc']<=0x1ff0000a and s['sp']==0x20010000)
 e.cmd('lcd ConfigureBoot false');e.cmd('machine Reset');e.run(.1);s=e.snapshot();check('Return to internal firmware boot',0x08000000<=s['pc']<0x08080000 or 0x90000000<=s['pc']<0x90800000)
finally:
 e.close();(ROOT/'docs/debug-boot-results.json').write_text(json.dumps(results,indent=2))
