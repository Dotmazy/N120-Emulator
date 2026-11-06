import pathlib,subprocess,sys,json,time
root=pathlib.Path(__file__).resolve().parents[1];result=[]
for n in range(3):
 start=time.perf_counter()
 with (root/'docs'/('usb-repeat-%d.log'%n)).open('w',encoding='utf-8') as log:run=subprocess.run([sys.executable,str(root/'tests/usb_lifecycle.py')],cwd=root,stdout=log,stderr=subprocess.STDOUT)
 result.append({'repeat':n,'exit':run.returncode,'wall_s':time.perf_counter()-start});print(result[-1],flush=True);(root/'docs/usb-repeat-results.json').write_text(json.dumps(result,indent=2))
 if run.returncode:sys.exit(run.returncode)
