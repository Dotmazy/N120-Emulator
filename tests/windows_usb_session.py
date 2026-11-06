"""Keep real firmware and loopback USB/IP alive for independent Windows tests.
Stop by creating build/stop-usb-session (bounded maximum 30 minutes).
"""
import argparse,json,pathlib,socket,time
from integration import Engine,ROOT
a=argparse.Namespace(renode='F:/Programs/Renode/renode.exe',internal=pathlib.Path('C:/Users/Dotmazy/Desktop/NEW/internal to test.bin'),external=pathlib.Path('C:/Users/Dotmazy/Desktop/NEW/external to test.bin'))
e=Engine(a);stop=ROOT/'build/stop-usb-session'
if stop.exists():stop.unlink()
try:
 e.machine(0x4e4101);e.run(5);e.cmd('lcd StartBridge 13241')
 with socket.create_connection(('localhost',13241)) as sock:
  stream=sock.makefile('rb')
  def cmd(c):sock.sendall((c+'\n').encode());return json.loads(stream.readline())
  for _ in range(3):cmd('key 0 4 1');e.run(.01);cmd('key 0 4 0');e.run(.1)
  e.cmd('start');cmd('cable 2');cmd('usbip 3240')
  print('READY USB/IP localhost:3240 bus 1-1; bridge 13241',flush=True)
  deadline=time.monotonic()+1800
  while time.monotonic()<deadline and not stop.exists():
   state=cmd('snapshot');state.pop('frame',None)
   (ROOT/'docs/windows-usb-session.json').write_text(json.dumps(state,indent=2));time.sleep(1)
  cmd('usbip-stop')
finally:e.close()
