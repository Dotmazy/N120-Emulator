import sys,pathlib,argparse,socket,json,base64,time
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parent))
from integration import Engine,ROOT,free_port
from integration import arguments
args=arguments()
e=Engine(args);results=[]
def check(name,passed,detail=None):
 results.append(dict(name=name,passed=bool(passed),detail=detail));print(('PASS ' if passed else 'FAIL ')+name+' '+str(detail),flush=True);assert passed,name
try:
 e.machine(0x4E4101);e.run(5);port=free_port();e.cmd('lcd StartBridge %d'%port)
 with socket.create_connection(('127.0.0.1',port)) as sock:
  f=sock.makefile('rb')
  def cmd(c):sock.sendall((c+'\n').encode());return json.loads(f.readline())
  def tap(k):cmd('key %d %d 1'%(k//6,k%6));e.run(.03);cmd('key %d %d 0'%(k//6,k%6));e.run(.03)
  def until(active):
   for _ in range(50):
    s=e.snapshot()
    if s['lcdActive']==active and (active or s['cpuWaiting']):return True
    e.run(.02)
   return False
  for _ in range(3):tap(4);e.run(.2)
  for cycle in range(12):
   tap(8);off=until(False)
   if cycle%3:e.run((.1,1)[cycle%2])
   tap(8);on=until(True);s=e.snapshot();check('Power sleep/wake cycle %d'%cycle,off and on and not s['faulted'] and not s['pressed'])
  # Check both SysTick inputs and clock division directly, independently of GUI pacing.
  e.write(0xE000E010,0);e.write(0xE000E014,1000);e.write(0xE000E010,3)
  check('SysTick divided reference selection',(e.read(0xE000E010)&4)==0 and e.number('nvic Divider')==8)
  e.write(0xE000E010,7);check('SysTick processor clock selection',(e.read(0xE000E010)&4)!=0 and e.number('nvic Divider')==1)
  cmd('pause');before=cmd('snapshot');bad=base64.b64encode(b'nonexistent-firmware.bin').decode();r=cmd('reload '+bad+' - 0 -');after=cmd('snapshot');check('invalid reload preserves paused firmware',not r['ok'] and before['pc']==after['pc'] and before['frame']==after['frame'])
  reload='reload '+base64.b64encode(str(args.internal).encode()).decode()+' '+base64.b64encode(str(args.external).encode()).decode()+' 0 -'
  elapsed=[]
  for cycle in range(3):
   t=time.perf_counter();r=cmd(reload);elapsed.append(time.perf_counter()-t);check('reload acknowledges and preserves pause %d'%cycle,r.get('reloaded') and cmd('snapshot')['paused']);e.run(5);s=e.snapshot();check('reloaded firmware boots %d'%cycle,s['lcdActive'] and s['gamma'] and not s['faulted'])
  check('in-process reload durations',True,elapsed)
  original=e.read(0x58024418);e.write(0x58024418,original&~0xF00);core=e.number('nvic Frequency');timer=e.number('timer3 Frequency');e.write(0x58024418,(original&~0xF00)|0xF00)
  check('RCC /512 core prescaler',e.number('nvic Frequency')==core//512)
  check('RCC /512 timer propagation',abs(e.number('timer3 Frequency')-timer//512)<=1);e.write(0x58024418,original)
  # Slow PWM must expose timer phase rather than its time-averaged brightness.
  e.write(0x58020400,2<<8);e.write(0x58020420,2<<16)
  for offset,value in [(0,0),(0x28,65535),(0x2C,9999),(0x18,6<<4),(0x20,1),(0x34,5000),(0x14,1),(0,1)]:e.write(0x40000400+offset,value)
  e.write(0x40000424,0);on=e.snapshot()['rgb'][0];e.write(0x40000424,9000);off=e.snapshot()['rgb'][0];check('slow LED PWM follows counter phase',on==1 and off==0,[on,off])
  e.write(0x40000400,0);e.write(0x40000418,5<<4);check('forced active LED output without running counter',e.snapshot()['rgb'][0]==1)
  cmd('shutdown');e.p.wait(5);check('shutdown after repeated reload',e.p.returncode==0)
finally:
 e.close();(ROOT/'docs/lifecycle-results.json').write_text(json.dumps(results,indent=2))
