"""ADB/UI smoke test against a paired host with user firmware already uploaded.

No root is needed. Set up pairing in Settings first. Uses native accessibility trees;
captures stay in ignored build output because skin/firmware redistribution rights are unresolved.
"""
import argparse
import json
import pathlib
import re
import subprocess
import time
import xml.etree.ElementTree as ET

ap=argparse.ArgumentParser()
ap.add_argument('--adb',required=True)
ap.add_argument('--serial',default='emulator-5554')
ap.add_argument('--output',type=pathlib.Path,default=pathlib.Path('build/android-test-output'))
args=ap.parse_args()
args.output.mkdir(parents=True,exist_ok=True)
package='org.n120.emulator'
activity=package+'/org.qtproject.qt.android.bindings.QtActivity'
def adb(*cmd):
    return subprocess.check_output([args.adb,'-s',args.serial,*cmd],stderr=subprocess.STDOUT).decode(errors='replace')
def tree():
    adb('shell','uiautomator','dump','/sdcard/n120-smoke.xml')
    text=adb('shell','cat','/sdcard/n120-smoke.xml')
    return list(ET.fromstring(text).iter('node'))
def label(node):return node.get('text') or node.get('content-desc','')
def tap(text,index=0):
    end=time.monotonic()+15
    nodes=[]
    while time.monotonic()<end:
        nodes=[n for n in tree() if label(n)==text and n.get('enabled')=='true' and n.get('bounds')!='[0,0][0,0]']
        if len(nodes)>index:break
        time.sleep(.3)
    if len(nodes)<=index:raise AssertionError('Cannot find enabled control: '+text)
    x1,y1,x2,y2=map(int,re.findall(r'\d+',nodes[index].get('bounds')))
    adb('shell','input','tap',str((x1+x2)//2),str((y1+y2)//2));time.sleep(.4)
def snapshot(timeout=40):
    end=time.monotonic()+timeout
    while time.monotonic()<end:
        for node in tree():
            value=label(node).strip()
            if value.startswith('{'):
                try:
                    data=json.loads(value)
                    if 'instructions' in data:return data
                except json.JSONDecodeError:pass
        time.sleep(.5)
    raise AssertionError('No real hardware snapshot received')
def capture(name):
    adb('shell','screencap','-p','/sdcard/n120-smoke.png')
    adb('pull','/sdcard/n120-smoke.png',str(args.output/(name+'.png')))

results={}
try:
    adb('shell','am','start','-n',activity);time.sleep(1)
    assert adb('shell','pidof',package).strip()
    results['startup']=True
    tap('Settings')
    current=tree()
    connect=[n for n in current if label(n)=='Connect' and n.get('enabled')=='true']
    if connect:tap('Connect')
    time.sleep(1)
    tap('Calculator')
    current=tree()
    if any(label(n)=='Start' and n.get('enabled')=='true' for n in current):tap('Start')
    elif any(label(n)=='Resume' and n.get('enabled')=='true' for n in current):tap('Resume')
    time.sleep(3)
    tap('Developer')
    data=snapshot()
    assert data['instructions']>1000 and data['lcdActive'] and data['pixelWrites']>1000
    results['actualRemoteFirmware']={k:data[k] for k in ('instructions','pc','sp','lcdActive','gamma','pixelWrites','virtualMs')}
    capture('developer-portrait')
    tap('Calculator');capture('calculator-portrait');tap('Developer')
    tap('Calculator');tap('Pause');tap('Developer')
    assert snapshot()['paused'];results['pause']=True
    tap('Calculator');tap('Resume');tap('Developer')
    assert not snapshot()['paused'];results['resume']=True
    before=snapshot()['instructions']
    adb('shell','settings','put','system','accelerometer_rotation','0')
    adb('shell','settings','put','system','user_rotation','1');time.sleep(1)
    capture('developer-landscape')
    assert snapshot()['instructions']>=before;results['rotationPreservesSession']=True
    adb('shell','settings','put','system','user_rotation','0');time.sleep(1)
    adb('shell','input','keyevent','KEYCODE_HOME');time.sleep(12)
    adb('shell','am','start','-n',activity);time.sleep(5)
    # Connection lease expired while suspended. Foreground reconnect must recover a real snapshot.
    tap('Developer');assert snapshot()['instructions']>=before
    results['foregroundReconnect']=True
    results['totalPss']=adb('shell','dumpsys','meminfo',package)
    results['crashes']=adb('logcat','-d','-b','crash')
    assert package not in results['crashes']
finally:
    adb('shell','settings','put','system','user_rotation','0')
    adb('shell','settings','put','system','accelerometer_rotation','1')
    (args.output/'results.json').write_text(json.dumps(results,indent=2))
print(json.dumps({k:v for k,v in results.items() if k not in ('totalPss','crashes')},indent=2))
