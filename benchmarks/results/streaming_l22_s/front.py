import json,sys,glob,os,re,math
d=sys.argv[1]; ctrl=sys.argv[2]; ref_np=sys.argv[3]
NPS=[400,500,600,700,800,1000,1300,1600]
def load(d):
    rows={}
    for f in sorted(glob.glob(d+'/*_op*.json')):
        m=re.match(r'(.+)_op(\d+)\.json',os.path.basename(f)); rows.setdefault(m.group(1),{})[int(m.group(2))]=json.load(open(f))
    return rows
def pts(j):
    return sorted((j['profile_np%d'%n]['blocks'],j['recall_np%d'%n],j['search_np%d_s'%n]) for n in NPS if 'recall_np%d'%n in j)
def interp(p,b):
    for (b0,r0,_),(b1,r1,_) in zip(p,p[1:]):
        if b0<=b<=b1: return r0+(r1-r0)*(math.log(b)-math.log(b0))/(math.log(b1)-math.log(b0))
    return p[0][1] if b<p[0][0] else p[-1][1]
def blocks_for(p,r):
    for (b0,r0,_),(b1,r1,_) in zip(p,p[1:]):
        if r0<=r<=r1: return math.exp(math.log(b0)+(math.log(b1)-math.log(b0))*(r-r0)/(r1-r0))
    return float('nan')
rows=load(d); c=rows[ctrl]
print('recall at control np%s blocks | blocks (M) to reach .998 | to reach .999 ; per state 200/1172/648, then mean recall'%ref_np)
for lab,r in rows.items():
    out=[];rs=[]
    for op in (200,1172,648):
        if op not in r or op not in c: out.append('-'); continue
        B=c[op]['profile_np%s'%ref_np]['blocks']; p=pts(r[op])
        rr=interp(p,B); rs.append(rr)
        out.append('%.4f (%.0fM/%.0fM/%.0fM)'%(rr,B/1e6,blocks_for(p,.998)/1e6,blocks_for(p,.999)/1e6))
    lj=r.get(648,{}).get('lists',{})
    print('%-6s'%lab,' | '.join(out),'| mean %.4f'%(sum(rs)/len(rs) if rs else 0),'| imb',lj.get('imb'))
