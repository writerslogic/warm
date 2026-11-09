import json,glob,sys,os
for f in sorted(glob.glob('G6/*_op*.json')):
    j=json.load(open(f)); b=os.path.basename(f)[:-5]
    bl={k[8:]:v['blocks']//1000 for k,v in j.items() if k.startswith('profile_np')}
    rc={k[7:]:j[k] for k in j if k.startswith('recall_np')}
    print('%-10s retrain %5.1fs rows %s imb %.2f max %d | '%(b,j.get('retrain_s',0),j.get('retrain_rows','-'),j['lists']['imb'],j['lists']['max'])+' '.join('%s=%.4f/%dk'%(k,rc[k],bl[k]) for k in rc))
