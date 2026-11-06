import sys,pathlib,argparse,socket,json,base64,time
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parent))
from integration import Engine,ROOT,free_port,png
from integration import arguments
args=arguments();e=Engine(args)
try:
 e.machine(0x4E4101);e.run(5);port=free_port();e.cmd('lcd StartBridge %d'%port)
 with socket.create_connection(('127.0.0.1',port)) as sock:
  f=sock.makefile('rb')
  def cmd(c):sock.sendall((c+'\n').encode());return json.loads(f.readline())
  def tap(i):cmd('key %d %d 1'%(i//6,i%6));e.run(.03);cmd('key %d %d 0'%(i//6,i%6));e.run(.2)
  for i in [4,4,4,6,2,2,2,3,4,2,2,2,2,2,4]:tap(i)
  s=cmd('snapshot');png(base64.b64decode(s['frame']),ROOT/'docs/python-shell-before.png')
  for i in [42,45,42,52]:tap(i)
  s=cmd('snapshot');png(base64.b64decode(s['frame']),ROOT/'docs/python-shell-two.png')
  # Physical Alpha sequences for sum(range(1000)); EXE is separate from typing.
  strokes=[13,37,13,39,13,30,33,13,36,13,18,13,31,13,24,13,22,33,42,48,48,48,34,34]
  cmd('sequence '+','.join(map(str,strokes)));e.run(3);before=cmd('snapshot');start=time.perf_counter();tap(52);after=cmd('snapshot');elapsed=time.perf_counter()-start
  png(base64.b64decode(after['frame']),ROOT/'docs/python-shell-result.png');result={'faulted':after['faulted'],'pending':after['sequencePending'],'pressed':after['pressed'],'evaluation_and_settle_wall_s':elapsed,'evaluation_and_settle_virtual_ms':after['virtualMs']-before['virtualMs'],'instructions':after['instructions']-before['instructions'],'pixels':after['pixelWrites']-before['pixelWrites']};(ROOT/'docs/python-shell-results.json').write_text(json.dumps(result,indent=2));print(result,flush=True);assert not after['faulted'] and not after['sequencePending'] and not after['pressed']
finally:e.close()
