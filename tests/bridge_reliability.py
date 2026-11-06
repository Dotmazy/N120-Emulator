"""Exercise actual bridge framing/reconnect behavior against Renode, without private source."""
import base64,json,socket,tempfile
from pathlib import Path
from integration import Engine,arguments,free_port
engine=Engine(arguments())
try:
 engine.machine(0x4e4101);engine.run(1)
 port=free_port();engine.cmd('lcd StartBridge %d'%port)
 with socket.create_connection(('127.0.0.1',port)) as client:
  client.sendall(b'x'*65537+b'\n')
  try:
   assert not client.recv(1), 'Oversized request was not rejected'
  except ConnectionResetError:pass
 with socket.create_connection(('127.0.0.1',port)) as client:
  client.sendall(b'snapshot\n')
  with client.makefile('rb') as stream:assert json.loads(stream.readline())['ok']
 print('PASS oversized command closes only that client; subsequent connection works')
 # Disconnect without consuming a response, then reconnect to the same listener.
 with socket.create_connection(('127.0.0.1',port)) as client:client.sendall(b'snapshot\n')
 with socket.create_connection(('127.0.0.1',port)) as client:
  client.sendall(b'snapshot\n')
  with client.makefile('rb') as stream:assert json.loads(stream.readline())['ok']
 print('PASS bridge survives a client disconnect during response')
 with tempfile.TemporaryDirectory() as directory:
  oversized=Path(directory)/'oversized.bin'
  with oversized.open('wb') as image:image.truncate(0x80001)
  token=base64.b64encode(str(oversized).encode()).decode()
  with socket.create_connection(('127.0.0.1',port)) as client:
   with client.makefile('rb') as stream:
    client.sendall(('reload '+token+' - 0 -\n').encode())
    assert not json.loads(stream.readline())['ok']
    client.sendall(b'snapshot\n')
    state=json.loads(stream.readline())
    assert state['ok'] and state['lcdActive'] and not state['faulted']
 print('PASS oversized reload is rejected without resetting the running firmware')

finally:engine.close()
