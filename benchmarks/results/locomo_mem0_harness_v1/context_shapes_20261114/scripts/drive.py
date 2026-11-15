"""drive.py PHASE CONTEXT CONVS DATASET CAP MAXK [extra harness args]: ingest-only, ingest-count gate, then answer and judge. Fresh dirs, no checkpoint reuse."""
import os, sys, subprocess, re, time, shutil, glob, threading
phase, context, convs, dataset, cap, maxk = sys.argv[1:7]; extra = sys.argv[7:]
H = os.path.expanduser('~'); P = '/Volumes/A/.hms-lb/locomo-20261114/context3'; W = H + '/ctx3-out'
OUT, STORE, IDX, SLOG = f'{W}/{phase}', f'{W}/{phase}-store', f'{W}/{phase}-index', f'{W}/{phase}-server.log'
def log(m): print(time.strftime('%H:%M:%S'), m, flush=True)
for d in (OUT, STORE, IDX): shutil.rmtree(d, ignore_errors=True); os.makedirs(d)
open(SLOG, 'w').close()
srv = subprocess.Popen(['/Volumes/C/.hms-tgt/locomo/release/pilot-memsrv', '--no-auth', '--bind', '127.0.0.1:0', '--data-dir', STORE + '/data', '--input-dim', '1536',
                        '--max-top-k', maxk, '--max-queue', '1000'], stdout=subprocess.PIPE, stderr=open(SLOG, 'ab'), env={'HOME': H, 'PATH': os.environ['PATH']})
m = re.search(r'listening on (\S+)', srv.stdout.readline().decode()); assert m, 'server did not start'
threading.Thread(target=lambda: [None for _ in srv.stdout], daemon=True).start()
host = f'http://{m.group(1)}'; log('server ' + host + f' max-top-k {maxk} context {context}')
env = {'HOME': H, 'PATH': os.environ['PATH'], 'PILOT_WORKERS': '16', 'PYTHONUNBUFFERED': '1', 'PILOT_CONVS': convs, 'WARM_HOST': host,
       'WARM_CONTEXT': context, 'WARM_INDEX_DIR': IDX, 'WARM_MAX_TOP_K': maxk, 'WARM_MERGE_GAP': '0', 'WARM_MERGE_MAX_TURNS': '300', 'WARM_MERGE_RECALL_K': '400'}
w = []
for p in 'openai tqdm httpx numpy tiktoken aiohttp aiolimiter python-dotenv pydantic requests'.split(): w += ['--with', p]
base = ['uv', 'run', '--no-project'] + w + ['python', '-I']
def run(args, logf): return subprocess.run(base + args, cwd=P, env=env, stdout=open(logf, 'ab'), stderr=subprocess.STDOUT).returncode
try:
    log('ingest only'); rc = run(['run_ctx.py', phase, cap, dataset, OUT, '--max-questions', '0'], f'{W}/{phase}-ingest.log'); log(f'ingest rc={rc}')
    if rc: sys.exit(rc)
    assert not glob.glob(OUT + '/**/_progress_*', recursive=True) or True
    rc = subprocess.run(['uv', 'run', '--no-project', '--with', 'numpy', '--with', 'openai', 'python', '-I', 'gate_ctx.py', OUT, SLOG, convs, f'{P}/{phase}-ingest-gate.txt'], cwd=P,
                        env={'HOME': H, 'PATH': os.environ['PATH'], 'WARM_HOST': host}, stdout=open(f'{W}/{phase}-gate.log', 'ab'), stderr=subprocess.STDOUT).returncode
    log(f'gate rc={rc}')
    if rc: sys.exit(rc)
    log('answer and judge'); rc = run(['run_ctx.py', phase, cap, dataset, OUT] + extra, f'{W}/{phase}-answer.log'); log(f'answer rc={rc}')
finally:
    srv.terminate(); log('server stopped')
