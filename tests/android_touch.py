"""Verify two real Android pointers reach Renode and cancel releases both.

The optional native local monitor is trusted test instrumentation on the Windows host,
not a network interface offered to the phone. Requires the compiled TouchInjector dex.
"""
import argparse
import json
import pathlib
import re
import socket
import subprocess
import time
import xml.etree.ElementTree as ET

ap=argparse.ArgumentParser()
ap.add_argument('--adb',required=True)
ap.add_argument('--monitor-port',required=True,type=int)
ap.add_argument('--dex',required=True,type=pathlib.Path)
ap.add_argument('--output',type=pathlib.Path,default=pathlib.Path('build/android-test-output'))
args=ap.parse_args()
args.output.mkdir(parents=True,exist_ok=True)
def adb(*command):return subprocess.check_output([args.adb,*command]).decode(errors='replace')
adb('push',str(args.dex),'/data/local/tmp/n120-touch.dex')
adb('shell','uiautomator','dump','/sdcard/n120-touch-ui.xml')
nodes=list(ET.fromstring(adb('shell','cat','/sdcard/n120-touch-ui.xml')).iter('node'))
def center(name):
    node=next(n for n in nodes if (n.get('text') or n.get('content-desc'))==name)
    x1,y1,x2,y2=map(int,re.findall(r'\d+',node.get('bounds')))
    return ((x1+x2)//2,(y1+y2)//2)

monitor=socket.create_connection(('127.0.0.1',args.monitor_port),timeout=5)
monitor.settimeout(.2)
try:monitor.recv(65536)
except TimeoutError:pass
def snapshot():
    monitor.sendall(b'lcd Snapshot\n')
    data=b'';end=time.monotonic()+5
    while time.monotonic()<end:
        try:data+=monitor.recv(262144)
        except TimeoutError:continue
        match=re.search(rb'(\{"ok":true,.*?\})',data,re.S)
        if match:return json.loads(match[1])
    raise TimeoutError('Renode monitor snapshot')

points=(*center('Shift'),*center('Alpha'))
helper=subprocess.Popen([args.adb,'shell','CLASSPATH=/data/local/tmp/n120-touch.dex',
    'app_process','/','TouchInjector',*map(str,points),'3000','cancel'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
time.sleep(.6)
try:
    held=snapshot()
    assert 12 in held['pressed'] and 13 in held['pressed'],held['pressed']
    adb('shell','screencap','-p','/sdcard/n120-touch.png')
    adb('pull','/sdcard/n120-touch.png',str(args.output/'touch-held.png'))
    output=helper.communicate(timeout=8)[0].decode()
    assert helper.returncode==0,output
    time.sleep(.2)
    released=snapshot()
    assert 12 not in released['pressed'] and 13 not in released['pressed'],released['pressed']
    adb('shell','screencap','-p','/sdcard/n120-touch.png')
    adb('pull','/sdcard/n120-touch.png',str(args.output/'touch-released.png'))
    results={'simultaneousPhysicalKeys':held['pressed'],'cancelReleasesKeys':released['pressed'],
             'instructions':released['instructions'],'lcdActive':released['lcdActive']}
    (args.output/'touch-results.json').write_text(json.dumps(results,indent=2))
    print(json.dumps(results,indent=2))
finally:
    monitor.close()
    if helper.poll() is None:helper.kill();helper.wait()
