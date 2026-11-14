import os, sys, subprocess, re, time, shutil, glob, threading, json
H = os.path.expanduser('~'); P = '/Volumes/A/.hms-lb/locomo-20261114/pilot'
OUT = H + '/pilot-out/all'; STORE = H + '/pilot-out/store'
DATA = '/Volumes/A/.hms-lb/locomo-20261114/locomo10.json'
def log(m): print(time.strftime('%H:%M:%S'), m, flush=True)
if not os.environ.get('KEEP'):
    shutil.rmtree(OUT, ignore_errors=True); shutil.rmtree(STORE, ignore_errors=True); open(H + '/pilot-out/all-server.log', 'w').close()
os.makedirs(OUT, exist_ok=True); os.makedirs(STORE, exist_ok=True)
if not os.environ.get('KEEP'): assert not glob.glob(OUT + '/**/_progress_*', recursive=True) + glob.glob(OUT + '/**/_ingestion_*', recursive=True)
srv = subprocess.Popen(['/Volumes/C/.hms-tgt/locomo/release/pilot-memsrv', '--no-auth', '--bind', '127.0.0.1:0', '--data-dir', STORE + '/data',
                        '--input-dim', '1536', '--max-top-k', '200', '--max-queue', '1000'], stdout=subprocess.PIPE, stderr=open(H + '/pilot-out/all-server.log', 'ab'))
m = re.search(r'listening on (\S+)', srv.stdout.readline().decode()); assert m, 'server did not start'
threading.Thread(target=lambda: [None for _ in srv.stdout], daemon=True).start()
host = f'http://{m.group(1)}'; log('server ' + host)
env = {'HOME': H, 'PATH': os.environ['PATH'], 'PILOT_WORKERS': '16', 'PYTHONUNBUFFERED': '1', 'PILOT_CONVS': '0,1,2,3,4,5,6,7,8,9', 'WARM_HOST': host}
w = []
for p in 'openai tqdm httpx numpy tiktoken aiohttp aiolimiter python-dotenv pydantic requests'.split(): w += ['--with', p]
base = ['uv', 'run', '--no-project'] + w + ['python', '-I']
def run(args, logf):
    return subprocess.run(base + args, cwd=P, env=env, stdout=open(logf, 'ab'), stderr=subprocess.STDOUT).returncode
try:
    log('phase 1: ingest only'); rc = run(['run_pilot.py', 'all', '45', DATA, OUT, '--max-questions', '0'], H + '/pilot-out/all-ingest.log'); log(f'ingest rc={rc}')
    if rc: sys.exit(rc)
    rc = subprocess.run(['uv', 'run', '--no-project', '--with', 'numpy', '--with', 'openai', 'python', '-I', 'gate.py', OUT], cwd=P, env={'HOME': H, 'PATH': os.environ['PATH'], 'WARM_HOST': host}, stdout=open(H + '/pilot-out/all-gate.log', 'ab'), stderr=subprocess.STDOUT).returncode
    log(f'gate rc={rc}')
    if rc: sys.exit(rc)
    log('phase 2: answer and judge'); rc = run(['run_pilot.py', 'all', '45', DATA, OUT], H + '/pilot-out/all-answer.log'); log(f'answer rc={rc}')
finally:
    srv.terminate(); log('server stopped')
