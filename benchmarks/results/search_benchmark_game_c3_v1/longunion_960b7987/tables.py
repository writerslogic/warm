import json,sys,statistics as st
path=sys.argv[1]; cmds=sys.argv[2].split(','); engines=sys.argv[3].split(',')
R=json.load(open(path))['results']
cats=['ALL','union','union:num_tokens_2','union:num_tokens_3','union:num_tokens_>3','intersection','intersection_union','phrase','negated','term']
for c in cmds:
    if c not in R: continue
    print(f'\n### {c} (mean / median us of per-query best of 10)\n')
    print('| category | n | '+' | '.join(engines)+' |'); print('|---|---|'+'---|'*len(engines))
    D={e:{q['query']:(q['tags'],min(q['duration'])) for q in R[c][e]} for e in engines}
    for cat in cats:
        row=[]
        n=0
        for e in engines:
            v=[d for t,d in D[e].values() if cat=='ALL' or cat in t]
            n=len(v)
            row.append(f'{st.mean(v):.0f} / {st.median(v):.0f}' if v else '-')
        if n: print(f'| {cat} | {n} | '+' | '.join(row)+' |')
