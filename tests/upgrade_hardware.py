import sys,pathlib,socket,json,argparse,base64,time
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parent))
from integration import Engine,ROOT,free_port,png
parser=argparse.ArgumentParser();parser.add_argument('--renode',default='F:/Programs/Renode/renode.exe');parser.add_argument('--internal',type=pathlib.Path,default=pathlib.Path('C:/Users/Dotmazy/Desktop/NEW/internal to test.bin'));parser.add_argument('--external',type=pathlib.Path,default=pathlib.Path('C:/Users/Dotmazy/Desktop/NEW/external to test.bin'));a=parser.parse_args()
e=Engine(a);results=[]
def check(name,passed,detail=''):
 results.append(dict(name=name,passed=bool(passed),detail=str(detail)));print(('PASS ' if passed else 'FAIL ')+name+' '+str(detail),flush=True)
 if not passed:raise AssertionError(name)
try:
 e.machine(0x4E4101);e.run(5);port=free_port();e.cmd('lcd StartBridge %d'%port)
 with socket.create_connection(('127.0.0.1',port)) as sock:
  with sock.makefile('rb') as stream:
   def command(c):sock.sendall((c+'\n').encode());r=json.loads(stream.readline());assert r['ok'],r;return r
   def key(i,down):command('key %d %d %d'%(i//6,i%6,down))
   def tap(i):key(i,1);e.run(.005);key(i,0);e.run(.15)
   for i in range(3):tap(4)
   check('initial LCD/backlight ready',e.snapshot()['backlight']==1 and e.snapshot()['lcdActive'])
   tap(8);s=e.snapshot();check('Power switch sleeps real firmware',not s['lcdActive'] and s['backlight']==0 and not s['faulted'])
   tap(8);s=e.snapshot();check('Power switch wakes real firmware',s['lcdActive'] and s['backlight']==1 and 0x90000000<=s['pc']<0x90800000)
   tap(8);tap(8);check('repeated power cycle',e.snapshot()['lcdActive'])
   key(8,1);e.run(.35);held=e.snapshot();key(8,0);e.run(.15);sleeping=e.snapshot();tap(8);awake=e.snapshot()
   check('held Power press/release and wake',8 in held['pressed'] and not held['faulted'] and not sleeping['lcdActive'] and awake['lcdActive'])
   for percent in (0,20,70,100):
    command('battery %d'%percent);s=e.snapshot();raw=e.read(0x40022040,'Word');check('battery ADC %d%%'%percent,abs(raw-round(s['batteryVoltage']/5*65535))<=1,(s['batteryVoltage'],raw))
   command('battery 70')
   for mode in (0,1,2,0):
    command('cable %d'%mode);s=e.snapshot();pa=e.read(0x58020010);pc=e.read(0x58020810);check('cable electrical signals %d'%mode,bool(pa&(1<<9))==(mode!=0) and bool(pc&(1<<10))==(mode==0))
   # Virtual-time physical key sequence: no text or event injection into firmware.
   command('sequence 13,23,13,26,13,44,13,23');e.run(1);s=e.snapshot();check('key sequence queue drains',s['sequencePending']==0 and not s['pressed'])
   png(base64.b64decode(s['frame']),ROOT/'docs/typed-keys.png')
   command('key 3 1 1');command('sequence 19');e.run(.15);held=e.snapshot();command('key 3 1 0');e.run(.03)
   check('queued tap preserves a directly held key',19 in held['pressed'] and 19 not in e.snapshot()['pressed'])
   # Exercise pacing against wall time while the global scheduler is live.
   e.cmd('start')
   for target in (.25,1,2):
    command('pause');command('speed %s'%target);start=e.snapshot()['virtualMs'];t=time.monotonic();command('resume');time.sleep(.65);command('pause');elapsed=time.monotonic()-t;delta=e.snapshot()['virtualMs']-start;ratio=delta/1000/elapsed
    check('speed target %sx'%target,ratio>target*.45 and ratio<target*1.35,round(ratio,3))
   command('speed 0');command('cable 2');command('resume');time.sleep(.2)
   descriptor=command('usb-descriptor');raw=base64.b64decode(descriptor.get('usbDescriptor',''));print('USB descriptor result',raw.hex(),flush=True)
   check('USB descriptor from real firmware',len(raw)==18 and raw[8:12]==bytes.fromhex('830491a2'),raw.hex())
   (ROOT/'docs/usb-descriptor-test.json').write_text(json.dumps({'descriptor':raw.hex(),'verified':len(raw)==18}))
   command('pause');s=e.snapshot();check('USB test does not fault firmware',not s['faulted'])
   command('shutdown');e.p.wait(timeout=5);check('clean shutdown',e.p.returncode==0)
finally:
 e.close();(ROOT/'docs/upgrade-hardware-results.json').write_text(json.dumps(results,indent=2))
