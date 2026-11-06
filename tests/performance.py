import argparse,json,time,pathlib,socket,struct,sys,os
from integration import Engine,ROOT,free_port
p=argparse.ArgumentParser();p.add_argument('--label',default='before');p.add_argument('--quick',action='store_true');opt=p.parse_args()
a=argparse.Namespace(renode='F:/Programs/Renode/renode.exe',internal=pathlib.Path('C:/Users/Dotmazy/Desktop/NEW/internal to test.bin'),external=pathlib.Path('C:/Users/Dotmazy/Desktop/NEW/external to test.bin'))
t=time.perf_counter();e=Engine(a);launch=time.perf_counter()-t;results={'launch_s':launch,'diagnostic_unprotected_xip':bool(os.environ.get('N120_PROFILE_NO_XIP_WRITE_WRAPPER'))}
try:
 t=time.perf_counter();e.machine(0x4E4101);results['machine_load_s']=time.perf_counter()-t;e.run(5)
 port=free_port();e.cmd('lcd StartBridge %d'%port)
 with socket.create_connection(('127.0.0.1',port)) as sock:
  with sock.makefile('rb') as f:
   def command(c):sock.sendall((c+'\n').encode());return json.loads(f.readline())
   def key(i,on):assert command('key %d %d %d'%(i//6,i%6,on))['ok']
   def tap(i):key(i,1);e.run(.03);key(i,0);e.run(.2)
   for i in [4,4,4,6]:tap(i)
   results['vector']=e.cmd('cpu VectorTableOffset')
   for name,held in [('idle',None),('held_down',2),('held_home',6)]:
    if held is not None:key(held,1)
    before=e.snapshot();t=time.perf_counter();e.run(1);wall=time.perf_counter()-t;s=e.snapshot()
    results[name]={'wall_s':wall,'virtual_ratio':1/wall,'instructions':s['instructions']-before['instructions'],'accesses':s['accesses']-before['accesses'],'pixels':s['pixelWrites']-before['pixelWrites'],'pc':hex(s['pc']),'cpuHz':s['cpuHz']}
    if 'profileMs' in s:results[name]['peripheral_ms']=s['profileMs']
    results[name]['timers']={hex(base):[e.read(base+offset,'Word') for offset in (0,0x0C,0x28,0x2C)] for base in (0x40000000,0x40000800,0x40001000)}
    if held is not None:key(held,0)
   if opt.quick:sys.exit(0)
   for cycle in range(5):
    tap(8);asleep=e.snapshot();e.run(.1);tap(8);awake=e.snapshot();results['power_'+str(cycle)]={'off':not asleep['lcdActive'],'on':awake['lcdActive'],'pc':hex(awake['pc'])}
   # All-mode cable cycles, then descriptor twice to expose stale connection state.
   e.cmd('start');command('cable 2');time.sleep(.2)
   results['descriptor1']=command('usb-descriptor');results['descriptor2']=command('usb-descriptor');command('cable 0');command('cable 2');time.sleep(.2);results['descriptor_replug']=command('usb-descriptor')
finally:
 e.close();(ROOT/('docs/performance-'+opt.label+'.json')).write_text(json.dumps(results,indent=2));print(json.dumps(results,indent=2),flush=True)
