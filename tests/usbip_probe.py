import sys,pathlib,socket,json,argparse,time,struct
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parent))
from integration import Engine,ROOT,free_port
from integration import arguments
a=arguments();e=Engine(a)
result={}
try:
 e.machine(0x4E4101);e.run(5);control=free_port();e.cmd('lcd StartBridge %d'%control)
 with socket.create_connection(('127.0.0.1',control)) as c:
  with c.makefile('rb') as f:
   c.sendall(b'cable 2\n');assert json.loads(f.readline())['ok'];e.cmd('start');time.sleep(.3)
   port=free_port();c.sendall(('usbip %d\n'%port).encode());response=json.loads(f.readline());print('server',response,flush=True);assert response['ok']
   with socket.create_connection(('127.0.0.1',port),timeout=10) as host:
    host.settimeout(10);host.sendall(struct.pack('>HHI',0x111,0x8005,0));reply=bytearray()
    while True:
     block=host.recv(4096)
     if not block:break
     reply.extend(block)
   result={'bytes':len(reply),'header':reply[:12].hex(),'reply':reply.hex()}
   print(result,flush=True)
   assert len(reply)>=12 and struct.unpack('>HHII',reply[:12])[1:]==(5,0,1)
   assert b'\x04\x83\xa2\x91' in reply
   print('PASS USB/IP exports actual firmware VID/PID',flush=True)
finally:
 e.close();(ROOT/'docs/usbip-probe.json').write_text(json.dumps(result,indent=2))
