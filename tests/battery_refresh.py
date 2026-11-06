import sys,pathlib,argparse,socket,json,time,base64,zlib
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parent))
from integration import Engine,ROOT,free_port,png
args=argparse.Namespace(renode='F:/Programs/Renode/renode.exe',internal=pathlib.Path('C:/Users/Dotmazy/Desktop/NEW/internal to test.bin'),external=pathlib.Path('C:/Users/Dotmazy/Desktop/NEW/external to test.bin'))
e=Engine(args);results=[]
try:
 e.machine(0x4E4101);e.run(5);port=free_port();e.cmd('lcd StartBridge %d'%port)
 with socket.create_connection(('127.0.0.1',port)) as sock:
  f=sock.makefile('rb')
  def cmd(c):sock.sendall((c+'\n').encode());return json.loads(f.readline())
  for _ in range(3):cmd('key 0 4 1');e.run(.03);cmd('key 0 4 0');e.run(.2)
  for percent in (20,100):
   before=cmd('snapshot');cmd('battery %d'%percent);initial=cmd('snapshot');seen=None
   for _ in range(60):
    e.run(.25);s=cmd('snapshot')
    if s['batteryReads']>before['batteryReads'] and s['lastBatterySample']==s['batteryAdc']:seen=s;break
   result={'percent':percent,'adc':s['batteryAdc'],'firmware_reads':s['batteryReads']-before['batteryReads'],'refresh_virtual_ms':None if seen is None else seen['virtualMs']-initial['virtualMs'],'passed':seen is not None and not s['faulted']};results.append(result);print(result,flush=True);png(base64.b64decode(s['frame']),ROOT/('docs/battery-%d.png'%percent));assert result['passed']
finally:
 e.close();(ROOT/'docs/battery-refresh-results.json').write_text(json.dumps(results,indent=2))
