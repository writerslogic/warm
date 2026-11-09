import json,glob,sys,os
d=sys.argv[1] if len(sys.argv)>1 else 'G2'
rows=[]
for f in sorted(glob.glob(f'{d}/*_op*.json')):
    with open(f) as fh: j=json.load(fh)
    b=os.path.basename(f)[:-5]
    rec=' '.join('%s=%.4f'%(k[7:],v) for k,v in j.items() if k.startswith('recall_np'))
    blk=' '.join('%s=%dk'%(k[8:-0].split('_')[0],v['blocks']/1000) for k,v in j.items() if k.startswith('profile_np') and False)
    print('%-12s retrain %6.1fs (centres %5.1f tables %4.1f place %5.1f) imb %.2f max %d | %s'%(b,j.get('retrain_s',0),j.get('retrain_centres_s',0),j.get('retrain_tables_s',0),j.get('retrain_place_s',0),j['lists']['imb'],j['lists']['max'],rec))
