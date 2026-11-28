import json, statistics as st, sys
E=["warm-main-final","warm-new-final","warm-new-oldidx"]
def load(p):
    with open(p) as f: return json.load(f)["results"]
A=load("final-A.json"); B=load("final-B.json")
def med(res,cmd,eng):
    qs=res[cmd][eng]
    return st.median(st.median(q["duration"]) for q in qs), st.mean(st.mean(q["duration"]) for q in qs), len(qs)
print("per-command median of per-query median duration (us), A and B orders; ratio main/new (>1 = new faster)")
for cmd in A:
    row=[]
    for e in E:
        a=med(A,cmd,e); b=med(B,cmd,e)
        row.append((a[0],b[0],a[1],b[1]))
    f=lambda x: f"{x*1e6:.1f}" if x<1 else f"{x:.1f}"
    print(cmd, A[cmd][E[0]].__len__(), "queries")
    for e,r in zip(E,row): print("  ",e, "median A/B", f(r[0]),f(r[1]), "mean A/B", f(r[2]),f(r[3]))
    m,n,o=row
    print("   main/new median A %.3f B %.3f ; mean A %.3f B %.3f ; main/oldidx median A %.3f B %.3f ; mean A %.3f B %.3f"%(m[0]/n[0],m[1]/n[1],m[2]/n[2],m[3]/n[3],m[0]/o[0],m[1]/o[1],m[2]/o[2],m[3]/o[3]))
# count diffs new vs oldidx
for cmd in ["COUNT","TOP_100_COUNT"]:
    for name,R in (("A",A),("B",B)):
        a={q["query"]:q["count"] for q in R[cmd]["warm-new-final"]}
        b={q["query"]:q["count"] for q in R[cmd]["warm-new-oldidx"]}
        c={q["query"]:q["count"] for q in R[cmd]["warm-main-final"]}
        print(cmd,name,"new vs oldidx differing:",sum(a[k]!=b[k] for k in a),"new vs main differing:",sum(a[k]!=c[k] for k in a),"of",len(a))
