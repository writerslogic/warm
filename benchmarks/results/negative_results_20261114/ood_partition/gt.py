import numpy as np, time
D="/home/davidcondrey/ood/up/data/text2image1B/"
def fb(fn,n=None):
    h=np.fromfile(fn,dtype=np.uint32,count=2); n=h[0] if n is None else n
    return np.memmap(fn,dtype=np.float32,mode="r",offset=8,shape=(n,h[1]))
X=np.array(fb(D+"base.1B.fbin.crop_nb_10000000",1000000)); Q=np.array(fb(D+"query.public.100K.fbin"))
out=np.zeros((len(Q),10),np.int64); t=time.time()
for s in range(0,len(Q),200):
    S=Q[s:s+200]@X.T
    p=np.argpartition(-S,10,axis=1)[:,:10]
    v=np.take_along_axis(S,p,1); o=np.argsort(-v,1)
    out[s:s+200]=np.take_along_axis(p,o,1)
    if s%10000==0: print(s,time.time()-t,flush=True)
np.save("/home/davidcondrey/oodpart/gt1m_100k.npy",out)
perm=np.random.default_rng(7).permutation(len(Q))
old=np.load("/home/davidcondrey/ood/gt_1000000_20000.npy")
print("agree_with_old_sets", np.mean([len(set(out[perm[i]])&set(old[i][:10]))/10 for i in range(20000)]))
print("DONE")
