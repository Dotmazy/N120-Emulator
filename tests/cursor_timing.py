import sys,pathlib,socket,json,argparse,base64
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]/'tests'))
from integration import Engine,ROOT,free_port,png
a=argparse.Namespace(renode='F:/Programs/Renode/renode.exe',internal=pathlib.Path('C:/Users/Dotmazy/Desktop/NEW/internal to test.bin'),external=pathlib.Path('C:/Users/Dotmazy/Desktop/NEW/external to test.bin'));e=Engine(a)
try:
 e.machine(0x4E4101);e.run(5);port=free_port();e.cmd('lcd StartBridge %d'%port)
 with socket.create_connection(('127.0.0.1',port)) as sock:
  with sock.makefile('rb') as stream:
   def command(c):sock.sendall((c+'\n').encode());assert json.loads(stream.readline())['ok']
   def tap(k):command('key %d %d 1'%(k//6,k%6));e.run(.03);command('key %d %d 0'%(k//6,k%6));e.run(.2)
   def screen(name):s=e.snapshot();png(base64.b64decode(s['frame']),ROOT/('docs/'+name+'.png'));print(name,hex(s['pc']),s['faulted'],flush=True)
   for k in [4,4,4,6,2,2,2,3,4]:tap(k)
   screen('python-app')
   tap(4);screen('python-editor-before')
   import zlib,time
   e.run(.5);last=None;changes=[];samples=[]
   for n in range(81):
    e.run(.05);s=e.snapshot();frame=base64.b64decode(s['frame']);body=frame[320*2*25:];crc=zlib.crc32(body)
    if last is not None and crc!=last:changes.append(round(n*.05,3))
    last=crc;samples.append({'virtual':s['virtualMs'],'cpu':s['cpuHz'],'pixels':s['pixelWrites'],'crc':crc})
   screen('python-cursor-final');result={'changes':changes,'samples':samples};(ROOT/'docs/cursor-timing.json').write_text(json.dumps(result,indent=2));print('cursor transitions',changes,flush=True)
   assert 5<=len(changes)<=9,changes
   assert all(.49<=b-a<=.66 for a,b in zip(changes,changes[1:])),changes
finally:e.close()
