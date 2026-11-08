"""Predicts the runbook's mean recall and search seconds from static-state curves (held-out).
usage: model.py DIR CONFIG [CTRL_DIR CTRL_CONFIG]
Assumptions (stated, not measured): blocks scale with live size L at fixed np (checked 3M->10M within 5%);
recall at L < 3M uses the 3M curve; steps with L < 1M are exact (recall 1, cost ~0); search time =
tau*blocks + fixed, calibrated so control np600 sums to 2250 s with fixed=265 s."""
import json,sys,math
NP=[400,600,800,1000,1300,1600]
ST={200:2999384,1172:6001880,648:9990953}
live=[a for a,b in json.load(open('/Volumes/A/.hms-lb/bigann/wl03-l22bench/runS/runbook_live.json'))]
def curves(d,cfg):
    out={}
    for op,L in ST.items():
        j=json.load(open(f'{d}/{cfg}_op{op}.json'))
        out[L]=[(n,j['profile_np%d'%n]['blocks'],j['recall_np%d'%n]) for n in NP if 'recall_np%d'%n in j]
    return out
def at(c,L):
    Ls=sorted(c)
    if L<=Ls[0]:
        s=L/Ls[0]; return [(n,b*s,r) for n,b,r in c[Ls[0]]]
    if L>=Ls[-1]: return c[Ls[-1]]
    for a,b in zip(Ls,Ls[1:]):
        if a<=L<=b:
            t=(L-a)/(b-a); return [(n,x1+(x2-x1)*t,r1+(r2-r1)*t) for (n,x1,r1),(_,x2,r2) in zip(c[a],c[b])]
TAU=1.0
TAUP=6.0e-8  # s per (query,list) table build, Ice Lake np600 run: 223 s / (600*10k*~620 steps)
def run(c,sched):
    tb=0;tr=0;m=0;tp=0
    for L in live:
        if L<1_000_000: tr+=1.0; m+=1; continue
        pts=at(c,L); n,b,r=sched(L,pts); tb+=b; tp+=n*2000; tr+=r; m+=1
    return (tb,tp),tr/m
def const(np_):
    return lambda L,pts:[p for p in pts if p[0]==np_][0]
def lagr(lam):
    return lambda L,pts:max(pts,key=lambda p:p[2]-lam*(p[1]*TAU+TAUP*p[0]*2000)*5/3e3)
if __name__=='__main__':
    d,cfg=sys.argv[1:3]
    cd,cc=(sys.argv[3:5] if len(sys.argv)>3 else (d,'K0g'))
    (cb,cp),_=run(curves(cd,cc),const(600)); TAU=(1905-223)/(cb*5)
    c=curves(d,cfg)
    print('config',cfg,'tau',TAU)
    for np_ in (400,600,800,1000,1300,1600):
        (b,p),r=run(c,const(np_)); print(' const np%-4d search %.0fs mean recall %.4f'%(np_,TAU*b*5+TAUP*p*5+265,r))
    print(' lagrangian schedule (best np per live size):')
    for lam in (1.6,3,6,12,24,48,96):
        (b,p),r=run(c,lagr(lam)); print('  lam %.4f search %.0fs mean recall %.4f'%(lam,TAU*b*5+TAUP*p*5+265,r))
