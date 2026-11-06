import pathlib,subprocess,sys,time,json
root=pathlib.Path(__file__).resolve().parents[1]
cases=['integration','upgrade_hardware','lifecycle','usb_lifecycle','battery_refresh','exam_led','cursor_timing']
if len(sys.argv)>1:cases=cases[cases.index(sys.argv[1]):]
results=[]
if len(sys.argv)>1 and (root/'docs/final-regressions.json').exists():results=[r for r in json.loads((root/'docs/final-regressions.json').read_text()) if r['suite'] not in cases]
for name in cases:
 start=time.perf_counter()
 with (root/'docs'/('final-'+name+'.log')).open('w',encoding='utf-8') as output:
  process=subprocess.run([sys.executable,str(root/'tests'/(name+'.py'))],cwd=root,stdout=output,stderr=subprocess.STDOUT)
 elapsed=time.perf_counter()-start
 results.append({'suite':name,'exit':process.returncode,'wall_s':elapsed});print(json.dumps(results[-1]),flush=True)
 (root/'docs/final-regressions.json').write_text(json.dumps(results,indent=2))
 if process.returncode:sys.exit(process.returncode)
