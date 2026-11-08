import json, numpy as np
O='/Volumes/A/.hms-lb/vibe-apex/out/'
import sys
GRID52=len(sys.argv)>1 and sys.argv[1]=='52'
new=json.load(open(O+('t2-llama-test-52.json' if GRID52 else 't1-llama-test.json'))); old=json.load(open(O+'t1-llama-test-oldgrid.json'))
def firstat(rows,t):
    q=[r['median_qps'] for r in rows if r['recall']>=t]; return max(q) if q else 0
base=['32:500:8:1','48:500:8:1','64:500:8:1','96:500:8:1']
elite=base+['24:500:8:1:0:0:64:100:0:253921','32:500:8:1:0:0:128:100:0:253921']
T=(0.95,0.99,0.995,0.998)
def union(d,names,t): return max(firstat(d['rows'][n],t) for n in names)
print('TEST llama full, 1000 official queries; best QPS over arms at first config reaching recall (no interpolation)')
print('control = 4 base builds on the OLD grid:', [int(union(old,base,t)) for t in T], 'max recall', round(max(max(r['recall'] for r in old['rows'][n]) for n in base),5))
print(('base builds on 52-point grid     :' if GRID52 else 'base builds on NEW grid          :'), [int(union(new,base,t)) for t in T], 'max recall', round(max(max(r['recall'] for r in new['rows'][n]) for n in base),5))
print(('elite (6 builds) on 52-point grid:' if GRID52 else 'elite (6 builds) on NEW grid      :'), [int(union(new,elite,t)) for t in T], 'max recall', round(max(max(r['recall'] for r in new['rows'][n]) for n in elite),5))
for n in elite:
    print(' ',n[:34].ljust(34),[int(firstat(new['rows'][n],t)) for t in T],'max',round(max(r['recall'] for r in new['rows'][n]),5))

if GRID52:
    c=union(old,base,0.99); print('check1 base-only >= control at 0.99:', int(union(new,base,0.99)), '>=', int(c), union(new,base,0.99)>=c)
    print('check2 elite/control at 0.99:', round(union(new,elite,0.99)/c,3), '>= 1.30', union(new,elite,0.99)/c>=1.30)
    print('ratios elite/control', [round(union(new,elite,t)/union(old,base,t),3) for t in T])
