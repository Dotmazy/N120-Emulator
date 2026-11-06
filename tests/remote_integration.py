"""Real-firmware, pinned-TLS host integration. Requires private user inputs, not bundled firmware."""
import argparse
import base64
import hashlib
import json
import pathlib
import socket
import ssl
import struct
import subprocess
import tempfile
import time

ap = argparse.ArgumentParser()
for name in ('host-exe', 'renode', 'cert', 'key', 'token-file', 'internal', 'external'):
    ap.add_argument('--' + name, type=pathlib.Path, required=True)
ap.add_argument('--output', type=pathlib.Path, default=pathlib.Path('build/remote-integration.json'))
args = ap.parse_args()
token = args.token_file.read_text().strip()
pin = hashlib.sha256(ssl.PEM_cert_to_DER_cert(args.cert.read_text())).hexdigest()
with socket.socket() as allocator:
    allocator.bind(('127.0.0.1', 0))
    port = allocator.getsockname()[1]
context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
context.check_hostname = False
context.verify_mode = ssl.CERT_NONE

class Client:
    def __init__(self, secret=token):
        self.socket = context.wrap_socket(socket.create_connection(('127.0.0.1', port), timeout=40))
        assert hashlib.sha256(self.socket.getpeercert(binary_form=True)).hexdigest() == pin
        self.send(type='auth', token=secret)
    def send(self, **packet):
        wire = json.dumps(dict(v=1, **packet), separators=(',', ':')).encode()
        self.socket.sendall(struct.pack('>I', len(wire)) + wire)
    def exact(self, size):
        result = bytearray()
        while len(result) < size:
            block = self.socket.recv(size-len(result))
            if not block:
                raise EOFError('Connection closed')
            result.extend(block)
        return bytes(result)
    def receive(self):
        length = struct.unpack('>I', self.exact(4))[0]
        assert 2 <= length <= 12*1024*1024
        return json.loads(self.exact(length))
    def until(self, kind, condition=lambda p: True, timeout=45):
        end = time.monotonic()+timeout
        while time.monotonic() < end:
            self.send(type='ping')
            packet = self.receive()
            if packet['type'] == kind and condition(packet):
                return packet
        raise TimeoutError(kind)
    def close(self):
        self.socket.close()

results = {}
args.output.parent.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix='n120-remote-', dir=args.output.parent) as storage:
    with (args.output.parent/'remote-host.txt').open('w') as log:
        host = subprocess.Popen([str(args.host_exe.resolve()), '--renode', str(args.renode.resolve()),
            '--storage', storage, '--cert', str(args.cert.resolve()), '--key', str(args.key.resolve()),
            '--token-file', str(args.token_file.resolve()), '--port', str(port)], stdout=log, stderr=log)
        client = None
        try:
            for _ in range(100):
                try:
                    with socket.create_connection(('127.0.0.1',port),timeout=.2): pass
                    break
                except OSError: time.sleep(.1)
            time.sleep(.3)
            bad = Client('x'*64)
            try: bad.receive(); raise AssertionError('Wrong token accepted')
            except (EOFError, OSError): results['rejectWrongToken'] = True
            finally: bad.close()
            time.sleep(.2)
            client = Client()
            welcome = client.until('welcome')
            assert welcome['execution']=='remote' and welcome['lcdWidth']==320
            results['pinnedTlsAuthentication'] = True
            client.send(type='control',command='gdb-start 3333')
            client.until('error')
            results['rejectRemoteDebugger'] = True
            for kind, path in [('internal',args.internal),('slotA',args.external)]:
                data = path.read_bytes()
                client.send(type='import',kind=kind,data=base64.b64encode(data).decode())
                reply = client.until('imported')
                assert reply['sha256'] == hashlib.sha256(data).hexdigest()
            results['firmwareUpload'] = True
            client.send(type='start',lcdId=0x4E4101)
            packet = client.until('snapshot',lambda p:p['data'].get('lcdActive') and p['data'].get('pixelWrites',0)>1000,timeout=65)
            snapshot = packet['data']
            assert snapshot['instructions']>100 and snapshot['pc']>=0x08000000
            results['firmwareBoot']={k:snapshot[k] for k in ('pc','sp','instructions','virtualMs','pixelWrites','lcdActive','gamma')}
            client.send(type='control',command='battery 37')
            client.until('snapshot',lambda p:p['data']['batteryPercent']==37)
            client.send(type='control',command='cable 1')
            client.until('snapshot',lambda p:p['data']['connectionMode']==1)
            results['batteryPower'] = True
            client.send(type='control',command='key 2 0 1')
            client.until('snapshot',lambda p:12 in p['data']['pressed'])
            client.send(type='control',command='key 2 0 0')
            client.until('snapshot',lambda p:12 not in p['data']['pressed'])
            results['matrixInput'] = True
            client.send(type='control',command='pause')
            client.until('snapshot',lambda p:p['data']['paused'])
            client.send(type='control',command='resume')
            client.until('snapshot',lambda p:not p['data']['paused'])
            results['pauseResume'] = True
            client.send(type='exportFlash')
            backup = client.until('flash')
            assert len(base64.b64decode(backup['data']))==0x800000
            results['flashBackup'] = True
            client.send(type='control',command='key 1 2 1')
            client.until('snapshot',lambda p:8 in p['data']['pressed'])
            client.close(); client = None
            time.sleep(.4)
            client = Client(); client.until('welcome')
            client.until('snapshot',lambda p:p['data']['paused'] and not p['data']['pressed'])
            results['disconnectReleasesAndPauses'] = True
            before=client.until('snapshot',lambda p:p['data']['paused'])['data']['instructions']
            blocked=pathlib.Path(storage)/'reload-checkpoint.bin'
            blocked.mkdir()
            client.send(type='reload')
            client.until('error')
            after=client.until('snapshot',lambda p:p['data']['paused'])['data']['instructions']
            assert after==before
            blocked.rmdir()
            results['failedCheckpointPreservesSession'] = True
            client.send(type='reload')
            client.until('reloaded')
            client.until('snapshot',lambda p:p['data']['paused'])
            results['reload'] = True
            client.send(type='restart')
            client.until('state',lambda p:p['text'].startswith('Stopped'),timeout=20)
            client.until('snapshot',lambda p:p['data'].get('lcdActive'),timeout=65)
            results['restart'] = True
            client.send(type='stop')
            client.until('state',lambda p:p['text'].startswith('Stopped'),timeout=20)
            results['stop'] = True
        finally:
            if client:
                client.send(type='stop'); time.sleep(2); client.close()
            host.terminate(); host.wait(timeout=10)
            args.output.write_text(json.dumps(results,indent=2))
print(json.dumps(results,indent=2))
