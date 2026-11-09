import json,sys,glob,os,re
d=sys.argv[1]; nps=sys.argv[2].split(',') if len(sys.argv)>2 else ['400','600','1000']
rows={}
for f in sorted(glob.glob(d+'/*_op*.json')):
    m=re.match(r'(.+)_op(\d+)\.json',os.path.basename(f)); lab,op=m.group(1),int(m.group(2))
    with open(f) as fh: j=json.load(fh)
    rows.setdefault(lab,{})[op]=j
ops=[200,1172,648]
print('label  | '+' | '.join(f'op{op}: '+'/'.join(f'r{n}' for n in nps)+' t' for op in ops)+' | mean recall by np | lists(imb,max) at 648')
for lab,r in rows.items():
    cells=[]; means={n:[] for n in nps}
    for op in ops:
        j=r.get(op)
        if not j: cells.append('-'); continue
        cells.append('/'.join('%.4f'%j['recall_np'+n] for n in nps)+' %.1fs'%j['search_np%s_s'%nps[1]]+' km%.0f tp%.0f'%(j['kmeans_s'],j['train_place_s']))
        for n in nps: means[n].append(j['recall_np'+n])
    mm=' '.join('np%s=%.4f'%(n,sum(v)/len(v)) for n,v in means.items() if v)
    ls=r.get(648,{}).get('lists',{})
    print(lab,'|',' | '.join(cells),'|',mm,'|',ls.get('imb'),ls.get('max'))

print()
print('blocks(M)/recall by np at op648 and op200 (held-out):')
for lab,r in rows.items():
    for op in (200,648):
        j=r.get(op)
        if j: print(lab,op,' '.join('np%s:%.0fM/%.4f/%.2fs'%(n,j['profile_np'+n]['blocks']/1e6,j['recall_np'+n],j['search_np'+n+'_s']) for n in nps))
