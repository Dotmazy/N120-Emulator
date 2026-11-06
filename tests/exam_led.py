import sys,pathlib,argparse,socket,json,base64
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]/'tests'))
from integration import Engine,ROOT,free_port,png
from integration import arguments
a=arguments();e=Engine(a)
try:
 e.machine(0x4E4101);e.run(5);port=free_port();e.cmd('lcd StartBridge %d'%port)
 with socket.create_connection(('127.0.0.1',port)) as sock:
  with sock.makefile('rb') as f:
   def key(i,on):sock.sendall(('key %d %d %d\n'%(i//6,i%6,on)).encode());assert json.loads(f.readline())['ok']
   def tap(i):key(i,1);e.run(.03);key(i,0);e.run(.2)
   for i in [4,4,4,6]:tap(i)
   for phase in ('idle','held'):
    if phase=='held':key(2,1)
    e.run(.2);print(phase,'NVIC Freq',e.cmd('nvic Frequency'),'Systick',[hex(e.read(x)) for x in [0xE000E010,0xE000E014]],flush=True)
    for base in (0x40000000,0x40000400,0x40000800,0x40001000):print(hex(base),[hex(e.read(base+x)) for x in [0,4,8,0x0C,0x18,0x1C,0x20,0x28,0x2C,0x34,0x38,0x3C]],flush=True)
    v=e.read(0xE000ED08);print('vectors',[hex(e.read(v+4*i)) for i in [15,16+28,16+30,16+54]],flush=True)
   key(2,0)
   for i in [6,2,2,2,3,3,4]:tap(i)
   s=e.snapshot();png(base64.b64decode(s['frame']),ROOT/'docs/settings-firmware.png')
   for i in range(7):tap(2)
   tap(4)
   for i in range(20):tap(1)
   s=e.snapshot();png(base64.b64decode(s['frame']),ROOT/'docs/country-first.png')
   tap(2);tap(4);tap(2);tap(4);s=e.snapshot();png(base64.b64decode(s['frame']),ROOT/'docs/france-exam-menu.png')
   tap(4);s=e.snapshot();png(base64.b64decode(s['frame']),ROOT/'docs/france-exam-confirm.png')
   tap(3);tap(4);e.run(1);s=e.snapshot();png(base64.b64decode(s['frame']),ROOT/'docs/france-exam-active.png')
   data=[]
   for n in range(40):
    e.run(.1);s=e.snapshot();data.append({'time':s['virtualMs'],'rgb':s['rgb'],'tim3':[hex(e.read(0x40000400+x)) for x in [0,4,8,0x18,0x1C,0x20,0x24,0x28,0x2C,0x34,0x38,0x3C]],'tim1':[hex(e.read(0x40010000+x)) for x in [0,4,8,0x18,0x1C,0x20,0x24,0x28,0x2C,0x34,0x38,0x3C]]});print(data[-1],flush=True)
   (ROOT/'docs/exam-led-samples.json').write_text(json.dumps(data,indent=2))
   assert 0<len([v for v in data if v['rgb'][0]>.9])<len(data)//2
   assert all(v['rgb'][1:]==[0,0] for v in data)
   assert (e.number('timer1 ProgrammedControl2')>>4)&7==4
   assert all(int(v['tim3'][6],16)==0 for v in data if v['rgb'][0]==0)
   print('PASS real French exam LED has red bursts, dark gaps and stopped slave counter',flush=True)
finally:e.close()
