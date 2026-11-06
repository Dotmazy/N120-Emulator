"""Real firmware USB/IP validation, distinct responses and cancellation recovery."""
import argparse,json,pathlib,socket,struct,time
from integration import Engine,ROOT,free_port
from integration import arguments
a=arguments()
results=[]
def check(name,value):
 print(('PASS ' if value else 'FAIL ')+name,flush=True);results.append({'name':name,'passed':bool(value)});assert value,name
def read(s,n):
 data=b''
 while len(data)<n:
  part=s.recv(n-len(data))
  if not part:raise EOFError()
  data+=part
 return data
e=Engine(a)
try:
 e.machine(0x4e4101);e.run(5);port=free_port();e.cmd('lcd StartBridge %d'%port)
 with socket.create_connection(('localhost',port)) as bridge:
  stream=bridge.makefile('rb')
  def cmd(c):bridge.sendall((c+'\n').encode());return json.loads(stream.readline())
  for _ in range(3):cmd('key 0 4 1');e.run(.01);cmd('key 0 4 0');e.run(.1)
  e.cmd('start');cmd('cable 2');port=free_port();cmd('usbip %d'%port)
  def connect(bus):
   s=socket.create_connection(('localhost',port));s.settimeout(8);s.sendall(struct.pack('>HHI',0x111,0x8003,0)+bus.encode().ljust(32,b'\0'));h=read(s,8);return s,h
  s,h=connect('not-the-emulator');check('Reject invalid import bus',struct.unpack('>I',h[4:])[0]==1);s.close()
  s,h=connect('1-1');read(s,312);check('Import accepted',struct.unpack('>I',h[4:])[0]==0)
  other,h=connect('1-1');check('Reject second owner',struct.unpack('>I',h[4:])[0]==1);other.close()
  sequence=0;directions={}
  def submit(setup,ep=0,payload=b''):
   global sequence
   sequence+=1;length=setup[6]|setup[7]<<8;direction=setup[0]>>7;directions[sequence]=direction
   s.sendall(struct.pack('>10I',1,sequence,0x10001,direction,ep,0,length,0,0xffffffff,0)+setup+payload)
   return sequence
  def response():
   h=read(s,48);status=struct.unpack('>i',h[20:24])[0];length=struct.unpack('>I',h[24:28])[0];return h,status,read(s,length) if length and directions.get(struct.unpack('>I',h[4:8])[0]) else b''
  submit(bytes.fromhex('8006000100001200'));h,status,d=response();check('Firmware device descriptor',status==0 and len(d)==18 and d[8:12]==bytes.fromhex('830491a2'))
  submit(bytes.fromhex('800601030904ff00'));h,status,d=response();check('Distinct manufacturer response',status==0 and d[2:].decode('utf-16-le')=='NumWorks')
  submit(bytes.fromhex('800602030904ff00'));h,status,d=response();check('Distinct product response',status==0 and d[2:].decode('utf-16-le')=='NumWorks Calculator')
  submit(bytes.fromhex('8006040309040100'));h,status,d=response();check('WebDFU interface string length probe',status==0 and d==bytes([72]))
  submit(bytes.fromhex('8006040309044800'));h,status,d=response();check('WebDFU 72-byte interface string spans EP0 packets',status==0 and len(d)==72 and d[2:].decode('utf-16-le').startswith('@Flash/'))
  submit(bytes.fromhex('800604030904ff00'));h,status,d=response();check('Multi-packet short response terminates 255-byte request',status==0 and len(d)==72)
  submit(bytes.fromhex('8000000000000200'));h,status,d=response();check('GET_STATUS is not stale string data',status==0 and d==b'\0\0')
  submit(bytes.fromhex('0009010000000000'));h,status,d=response();check('SET_CONFIGURATION reaches firmware',status==0)
  submit(bytes.fromhex('8008000000000100'));h,status,d=response();check('GET_CONFIGURATION firmware response',status==0 and d==b'\1')
  for _ in range(40):
   submit(bytes.fromhex('0009010000000000'));h,status,d=response();assert status==0,('Repeated SET_CONFIGURATION',status)
   submit(bytes.fromhex('8008000000000100'));h,status,d=response();assert status==0 and d==b'\1'
  check('Forty consecutive zero-length status/control cycles',True)
  submit(bytes.fromhex('2101000000000500'),payload=bytes.fromhex('2100000390'));h,status,d=response();check('DFU OUT payload reaches firmware',status==0 and struct.unpack('>I',h[24:28])[0]==5)
  for _ in range(2):submit(bytes.fromhex('a103000000000600'));h,status,d=response();assert status==0 and d[0]==0
  submit(bytes.fromhex('2106000000000000'));h,status,d=response();assert status==0
  submit(bytes.fromhex('a102020000001000'));h,status,d=response();check('DFU UPLOAD matches actual loaded image',status==0 and d==a.external.read_bytes()[0x30000:0x30010])
  submit(bytes.fromhex('a102020000000008'));h,status,d=response();check('2048-byte DFU upload completes all 32 IN packets',status==0 and d==a.external.read_bytes()[0x30000:0x30800])
  submit(bytes.fromhex('2106000000000000'));h,status,d=response();assert status==0
  submit(bytes.fromhex('8000000000000200'),ep=1);h,status,d=response();check('Unsupported endpoint reports failure with zero actual length',status==-32 and not d)
  started=time.monotonic();submit(bytes.fromhex('8006ffff00001000'));h,status,d=response();check('Firmware STALL returns promptly',status==-32 and time.monotonic()-started<1)
  e.cmd('cpu IsHalted true');target=submit(bytes.fromhex('8006000100001200'));time.sleep(.05)
  sequence+=1;s.sendall(struct.pack('>6I',2,sequence,0x10001,0,0,target)+bytes(24));h=read(s,48)
  check('UNLINK cancels pending firmware request',struct.unpack('>I',h[:4])[0]==4 and struct.unpack('>i',h[20:24])[0]==-104)
  e.cmd('cpu IsHalted false')
  nextSeq=submit(bytes.fromhex('8006000100001200'));h,status,d=response();check('No stale completion after UNLINK; next transfer recovers',struct.unpack('>I',h[4:8])[0]==nextSeq and status==0 and len(d)==18)
  check('USB/IP reply reserved basic fields are zero',h[8:20]==bytes(12));s.close();cmd('usbip-stop')
finally:
 e.close();(ROOT/'docs/usb-protocol-results.json').write_text(json.dumps(results,indent=2))
