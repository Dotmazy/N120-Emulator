"""Hardware contract and real-firmware tests, using Renode's supported Robot server.
Run with Python 3.13. No third-party Python packages are required.
"""
import argparse,base64,json,pathlib,re,socket,struct,subprocess,time,xmlrpc.client,os,shutil

ROOT=pathlib.Path(__file__).resolve().parents[1]
MODEL=pathlib.Path(os.environ.get('N120_MODEL_ROOT',ROOT/'renode'))
socket.setdefaulttimeout(40)
def arguments(parser=None):
 ap=parser or argparse.ArgumentParser(description='Requires user-supplied compatible firmware; never downloads images.')
 ap.add_argument('--renode',default=os.environ.get('RENODE_PATH') or shutil.which('renode'))
 ap.add_argument('--internal',type=pathlib.Path,default=os.environ.get('N120_INTERNAL'))
 ap.add_argument('--external',type=pathlib.Path,default=os.environ.get('N120_EXTERNAL'))
 args=ap.parse_args()
 for name in ('renode','internal','external'):
  value=getattr(args,name)
  if not value or not pathlib.Path(value).is_file():ap.error('Provide --'+name+' or its documented environment variable pointing to a readable file')
 return args

def free_port():
 with socket.socket() as s:
  s.bind(('127.0.0.1',0));return s.getsockname()[1]
def at(p):return '@'+str(p).replace('\\','/').replace(' ','\\ ')
def png(data,path):
 import zlib
 out=bytearray()
 for y in range(240):
  out.append(0)
  for x in range(320):
   p=struct.unpack_from('<H',data,2*(y*320+x))[0];out.extend(((p>>11)*255//31,((p>>5)&63)*255//63,(p&31)*255//31))
 def chunk(t,d):return struct.pack('>I',len(d))+t+d+struct.pack('>I',zlib.crc32(t+d))
 path.write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',320,240,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(out))+chunk(b'IEND',b''))

class Engine:
 def __init__(self,args):
  (ROOT/'build/test-sessions').mkdir(parents=True,exist_ok=True)
  self.args=args;self.port=free_port();self.log=open(ROOT/'docs/integration-renode.log','w',encoding='utf8')
  self.p=subprocess.Popen([args.renode,'--config',str(ROOT/'build/test-sessions/renode.config'),'--disable-gui','--hide-monitor','--robot-server-port',str(self.port)],stdout=self.log,stderr=subprocess.STDOUT)
  self.rpc=xmlrpc.client.ServerProxy('http://localhost:%d/'%self.port)
  for i in range(200):
   try:self.rpc.get_keyword_names();break
   except OSError:
    if self.p.poll() is not None:
     self.log.close();raise RuntimeError('Renode startup failed; see integration-renode.log')
    time.sleep(.05)
  else:
   self.p.kill();self.p.wait();self.log.close();raise TimeoutError('Renode Robot server startup timed out')
  try:
   self.cmd('include '+at(MODEL/'UsbHost.cs'))
   self.cmd('include '+at(MODEL/'N120.cs'))
  except Exception:
   self.close();raise
 def cmd(self,c):
  r=self.rpc.run_keyword('ExecuteCommand',[c])
  if r['status']!='PASS':raise RuntimeError(c+': '+r.get('error',''))
  return r.get('return','')
 def number(self,c):return int(self.cmd(c).strip(),0)
 def write(self,a,v,width='DoubleWord'):self.cmd('sysbus Write%s 0x%X 0x%X'%(width,a,v))
 def read(self,a,width='DoubleWord'):return self.number('sysbus Read%s 0x%X'%(width,a))
 def run(self,t):self.cmd('emulation RunFor "%s"'%t)
 def machine(self,lcd_id):
  self.rpc.run_keyword('ResetEmulation',[])
  self.cmd('mach create "N120"')
  p=ROOT/'build/test-sessions/test.repl'
  text=(MODEL/'n120.repl').read_text().replace('lcd: N120.Lcd @ sysbus 0x60000000\n    board: board','lcd: N120.Lcd @ sysbus 0x60000000\n    board: board\n    id: '+str(lcd_id))
  p.write_text(text)
  self.cmd('machine LoadPlatformDescription '+at(p))
  self.cmd('sysbus LoadBinary '+at(self.args.internal)+' 0x08000000')
  self.cmd('ospi Load "'+str(self.args.external).replace('\\','/')+'"')
  self.cmd('lcd ConfigureRevision '+str(self.read(0x0807FA00)))
  if not os.environ.get('N120_PROFILE_NO_XIP_WRITE_WRAPPER'):self.cmd('ospi ProtectMapping')
 def snapshot(self):return json.loads(self.cmd('lcd Snapshot').strip())
 def lcd(self,c,data=()):
  self.write(0x60000000,c,'Word')
  for v in data:self.write(0x60020000,v,'Word')
 def close(self):
  try:self.rpc.run_keyword('StopRemoteServer',[])
  except (OSError,xmlrpc.client.Error):pass
  try:self.p.wait(timeout=5)
  except subprocess.TimeoutExpired:self.p.kill();self.p.wait()
  self.log.close()

def main():
 a=arguments()
 (ROOT/'build/test-sessions').mkdir(parents=True,exist_ok=True)
 results=[]
 def check(name,condition,detail=''):
  results.append(dict(name=name,passed=bool(condition),detail=detail));print(('PASS ' if condition else 'FAIL ')+name+' '+detail,flush=True)
  if not condition:raise AssertionError(name)
 e=Engine(a)
 try:
  for id in [0x4E4101,0x4E4102,0x4E4105,0x4E4106,0x4E4601,0x4E4801,0x4E5401]:
   e.machine(id);e.run(1);s=e.snapshot()
   check('firmware gamma/init %06X'%id,s['gamma'] and s['lcdActive'] and s['pixelWrites']>76800 and not s['faulted'],str({k:v for k,v in s.items() if k!='frame'}))
   check('fault registers %06X'%id,e.read(0xE000ED28)==0 and e.read(0xE000ED2C)==0)
   pixels=s['pixelWrites']
   e.lcd(0xE0,[0]*14)
   e.lcd(0x2C,[0xFFFF])
   s=e.snapshot();check('reject wrong gamma %06X'%id,not s['gamma'] and not s['lcdActive'] and s['pixelWrites']==pixels)
   e.cmd('machine Reset');e.run(1);check('gamma recovery after firmware reboot %06X'%id,e.snapshot()['lcdActive'])
   e.lcd(1);s=e.snapshot();check('LCD software reset %06X'%id,not s['gamma'] and not s['lcdActive'])
  e.machine(0x4E4101);e.run(5)
  s=e.snapshot();check('external firmware executes',0x90000000<=s['pc']<0x90800000 and s['lcdActive'] and s['pixelWrites']>300000)
  before=base64.b64decode(s['frame']);beforeWrites=s['pixelWrites'];png(before,ROOT/'docs/language-before.png')
  bridge=free_port();e.cmd('lcd StartBridge %d'%bridge)
  with socket.create_connection(('127.0.0.1',bridge)) as connection:
   with connection.makefile('rb') as stream:
    def key(row,col,pressed):
     connection.sendall(('key %d %d %d\n'%(row,col,pressed)).encode());assert json.loads(stream.readline())['ok']
    key(0,2,1);e.run(.15);key(0,2,0);e.run(.15)
    s=e.snapshot();after=base64.b64decode(s['frame']);png(after,ROOT/'docs/language-after.png')
    check('firmware responds to physical Down key',before!=after and s['pixelWrites']>beforeWrites)
    e.cmd('start')
    connection.sendall(b'pause\n');assert json.loads(stream.readline())['ok']
    connection.sendall(b'resume\n');assert json.loads(stream.readline())['ok'];time.sleep(.05)
    connection.sendall(b'pause\n');assert json.loads(stream.readline())['ok']
    paused=e.snapshot();time.sleep(.03);later=e.snapshot()
    check('asynchronous pause/resume',paused['paused'] and later['virtualMs']==paused['virtualMs'] and paused['virtualMs']>s['virtualMs'],str((s['virtualMs'],paused['virtualMs'],later['virtualMs'])))
    e.cmd('pause')
    # All physical switches, with direct electrical scanning while CPU is paused.
    rowpins=[1,0,2,3,6,7,8,10,15]
    cols=[1,2,3,4,5,6]
    rowmask=sum(1<<p for p in rowpins)
    modes=sum(1<<(p*2) for p in rowpins)
    e.write(0x58020000,modes)
    e.cmd('lcd ConfigureRevision 0xFFFFFFFF')
    indices=list(range(0,7))+[8]+list(range(12,35))+list(range(36,41))+list(range(42,47))+list(range(48,53))
    for index in indices:
     row,col=divmod(index,6);e.write(0x58020014,rowmask&~(1<<rowpins[row]));key(row,col,1)
     assert not e.read(0x58020810)&(1<<cols[col]),('press',index)
     key(row,col,0);assert e.read(0x58020810)&(1<<cols[col]),('release',index)
    check('all 46 switches press/release',len(indices)==46)
    e.write(0x58020014,rowmask&~(1<<rowpins[5]));key(5,0,1);key(5,1,1)
    check('simultaneous key electrical scan',e.read(0x58020810)&((1<<1)|(1<<2))==0)
    key(5,0,0);key(5,1,0)
    # Effective PWM at timer register level, including output polarity and enable.
    e.write(0x58020400,(2<<8)|(2<<10));e.write(0x58020420,(2<<16)|(2<<20))
    for offset,value in [(0,0),(0x2C,99),(0x18,(6<<4)|(6<<12)),(0x20,0x11),(0x34,25),(0x38,75),(0,1)]:e.write(0x40000400+offset,value)
    rgb=e.snapshot()['rgb'];check('PWM effective 25/75 percent',rgb[:2]==[.25,.75],str(rgb))
    e.write(0x40000420,0);check('PWM disabled outputs off',e.snapshot()['rgb'][:2]==[0,0])
    e.write(0x58020800,2<<16);e.write(0x58020824,3)
    for offset,value in [(0,0),(0x2C,199),(0x1C,6<<4),(0x20,0x100),(0x3C,100),(0x44,1<<15),(0,1)]:e.write(0x40010400+offset,value)
    check('blue PWM 50 percent',e.snapshot()['rgb'][2]==.5)
  # Flash contract through the MCU OctoSPI registers at a spare scratch sector.
  address=0x7FF000;original=e.read(0x907FF000)
  e.write(0x907FF000,0x12345678);check('mapped writes rejected',e.read(0x907FF000)==original)
  def opcode(op,addr=None,data=None):
   e.write(0x52005000,1)
   e.write(0x52005040,0 if data is None else len(data)-1)
   e.write(0x52005100,1|(0x100 if addr is not None else 0)|(0x1000000 if data else 0))
   e.write(0x52005110,op)
   if addr is not None:e.write(0x52005048,addr)
   if data:
    for v in data:e.write(0x52005050,v,'Byte')
  opcode(6);opcode(0x20,address);check('4 KiB erase',e.read(0x907FF000)==0xFFFFFFFF)
  opcode(0x32,address,[0]);check('program requires WEL',e.read(0x907FF000)==0xFFFFFFFF)
  opcode(6);opcode(0x32,address,[0x5A,0xA5]);check('NOR program',e.read(0x907FF000)&65535==0xA55A)
  opcode(5);busy=e.read(0x52005050,'Byte');check('flash busy status',(busy&1)!=0)
  e.read(0x52005050,'Byte');check('flash busy completion',e.read(0x52005050,'Byte')&1==0)
  opcode(6);opcode(0x32,address,[0xFF,0xFF]);check('NOR cannot program zero back to one',e.read(0x907FF000)&65535==0xA55A)
  saved=ROOT/'build/test-sessions/persist.bin';e.cmd('ospi Save "'+saved.as_posix()+'"');check('flash save size',saved.stat().st_size==0x800000)
  opcode(6);opcode(0x20,address);e.cmd('ospi Load "'+saved.as_posix()+'"');check('flash persistence reload',e.read(0x907FF000)&65535==0xA55A)
  e.cmd('machine Reset');check('MCU reset retains flash',e.read(0x907FF000)&65535==0xA55A)
  e.run(1);s=e.snapshot();check('firmware reboot reinitializes LCD',s['lcdActive'] and s['gamma'] and s['pixelWrites']>76800 and not s['faulted'])
  with socket.create_connection(('127.0.0.1',bridge)) as connection:
   with connection.makefile('rb') as stream:
    for invalid in ('key -1 0 1','key 0 6 1','key 0 0 2','battery','reload','battery 999999999999999999999'):
     connection.sendall((invalid+'\n').encode());check('reject malformed command '+invalid,not json.loads(stream.readline())['ok'])
    connection.sendall(b'snapshot\n');check('bridge survives malformed commands',json.loads(stream.readline())['ok'])
    connection.sendall(b'shutdown\n');check('shutdown acknowledged',json.loads(stream.readline())['ok'])
  e.p.wait(timeout=5);check('Renode terminates cleanly',e.p.returncode==0)
 finally:
  e.close();(ROOT/'docs/test-results.json').write_text(json.dumps(results,indent=2))
 print('%d checks passed'%len(results),flush=True)
if __name__=='__main__':main()
