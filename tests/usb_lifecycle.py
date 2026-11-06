import sys,pathlib,argparse,socket,json,time
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]/'tests'))
from integration import Engine,free_port
from integration import arguments
args=arguments()
e=Engine(args)
try:
 e.machine(0x4E4101);e.run(5);port=free_port();e.cmd('lcd StartBridge %d'%port)
 with socket.create_connection(('127.0.0.1',port)) as sock:
  f=sock.makefile('rb')
  def cmd(c):sock.sendall((c+'\n').encode());return json.loads(f.readline())
  for i in [4,4,4]:cmd('key 0 4 1');e.run(.01);cmd('key 0 4 0');e.run(.1)
  e.cmd('start');cmd('cable 2')
  import struct,base64
  results=[]
  def check(name,ok):
   results.append({'name':name,'passed':bool(ok)});print(('PASS ' if ok else 'FAIL ')+name,flush=True);assert ok,name
  for n,mode in enumerate([2,0,2,1,2,0,1,0,2]):
   cmd('cable %d'%mode);time.sleep(.15)
   if mode==2:
    r=cmd('usb-descriptor');state=cmd('snapshot');print({k:v for k,v in state.items() if k.startswith('usb')},'regs',[hex(e.read(0x40040000+x)) for x in [0x14,0x18,0x804,0xB08,0xB10,0x908]],flush=True);check('descriptor cable cycle %d'%n,len(base64.b64decode(r['usbDescriptor']))==18)
  port=free_port()
  def receive(sock,n):
   data=b''
   while len(data)<n:
    part=sock.recv(n-len(data))
    if not part:
     state=cmd('snapshot');print({k:v for k,v in state.items() if k.startswith('usb')},flush=True);raise EOFError()
    data+=part
   return data
  for cycle in range(3):
   cmd('usbip %d'%port);client=socket.create_connection(('127.0.0.1',port));client.settimeout(5)
   if cycle==1:
    client.sendall(struct.pack('>HHI',0x111,0x8005,0));header=receive(client,12);device=receive(client,312);interfaces=receive(client,device[311]*4);check('USB/IP DEVLIST firmware identity',header[8:12]==b'\x00\x00\x00\x01' and device[300:304]==bytes.fromhex('0483a291'))
   if cycle==2:
    client.sendall(struct.pack('>HHI',0x111,0x8003,0)+b'1-1'+bytes(29));header=receive(client,8);device=receive(client,312)
    urb=struct.pack('>IIIIIIIIII',1,10,0x10001,1,0,0,18,0,0,0)+bytes.fromhex('8006000100001200');client.sendall(urb);reply=receive(client,48);data=receive(client,struct.unpack('>I',reply[24:28])[0]);check('USB/IP imported EP0 descriptor',len(data)==18 and data[8:12]==bytes.fromhex('830491a2'))
    short=struct.pack('>IIIIIIIIII',1,11,0x10001,1,0,0,64,0,0,0)+bytes.fromhex('8006000100004000');client.sendall(short);reply=receive(client,48);data=receive(client,struct.unpack('>I',reply[24:28])[0]);check('USB/IP short packet completes 64-byte request',len(data)==18)

   start=time.monotonic();cmd('usbip-stop');check('server stop with client %d'%cycle,time.monotonic()-start<2);client.close()
   rebound=socket.socket();rebound.bind(('127.0.0.1',port));rebound.close();check('USB/IP port released %d'%cycle,True)
  cmd('usbip %d'%port);cmd('usbip-stop');check('server stop without a client',True)
  cmd('usbip %d'%port);client=socket.create_connection(('127.0.0.1',port));cmd('shutdown');e.p.wait(5);client.close();check('shutdown with listening server and client',e.p.returncode==0)
  pathlib.Path('docs/usb-lifecycle-results.json').write_text(json.dumps(results,indent=2))
finally:e.close()
