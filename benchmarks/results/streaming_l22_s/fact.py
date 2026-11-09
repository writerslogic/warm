import json,sys
sys.argv=['x']
with open('front.py') as _fh: _src=_fh.read()
exec(_src.split("rows=load")[0].replace("d=sys.argv[1]; ctrl=sys.argv[2]; ref_np=sys.argv[3]",""))
def load1(path):
    with open(path) as fh: return json.load(fh)
ctrl={op:load1(f'G1/K0g_op{op}.json') for op in (200,1172,648)}
def rec(path,op):
    j=load1(path); B=ctrl[op]['profile_np600']['blocks']; return interp(pts(j),B), j['lists']['imb']
cells={'A0B0 (262k, stale)':'K/K3','A0B1 (262k, warm)':'W/W1','A1B0 (1M, stale)':'F/F1','A1B1 (1M, warm)':'F/F2'}
res={}
for name,p in cells.items():
    for op in (1172,648):
        try: res[(name,op)]=rec(f'{p}_op{op}.json',op)
        except Exception as e: res[(name,op)]=None
for op in (1172,648):
    print('state',op,'control (op32 centres)  %.4f'%rec(f'G1/K0g_op{op}.json',op)[0])
    v={k:res[(k,op)] for k in cells}
    for k,x in v.items(): print('  %-22s'%k, 'n/a' if x is None else 'recall %.4f imb %.2f'%x)
    if all(x is not None for x in v.values()):
        f=[v[k][0] for k in cells]; add=f[0]+(f[1]-f[0])+(f[2]-f[0]); print('  additive prediction A1B1 %.4f actual %.4f interaction %+.4f'%(add,f[3],f[3]-add))
