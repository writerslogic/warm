"""Open-weight ingest extraction on Modal: Qwen3-30B-A3B-Instruct-2507 (pinned revision), vLLM 0.11.0, greedy.
usage: uvx modal run extract_modal.py --inp ext_in_dev.jsonl --out ext_out_dev.jsonl --chunk 1200
Resumes: keys already in --out are skipped. Cost = GPU seconds * H100 rate * 1.15 (logged to stdout)."""
import json, time
from pathlib import Path
import modal
MODEL, REV = "Qwen/Qwen3-30B-A3B-Instruct-2507", "0d7cf23991f47feeb3a57ecb4c9cee8ea4a17bfe"
VOL = modal.Volume.from_name("warm-lme", create_if_missing=True)
image = (modal.Image.debian_slim(python_version="3.12").uv_pip_install("vllm==0.11.0", "transformers==4.57.1", "numpy<2.3")
         .env({"HF_HOME": "/vol/hf", "TOKENIZERS_PARALLELISM": "false"}))
app = modal.App("warm-lme-extract")

@app.function(gpu="H100", image=image, volumes={"/vol": VOL}, timeout=4 * 3600, max_containers=3)
def run(prompts: list[str]) -> tuple[list[str], float]:
    from vllm import LLM, SamplingParams
    t = time.time()
    llm = LLM(model=MODEL, revision=REV, max_model_len=12288, gpu_memory_utilization=0.9, enable_prefix_caching=True)
    tok = llm.get_tokenizer()
    chats = [tok.apply_chat_template([{"role": "user", "content": p}], tokenize=False, add_generation_prompt=True,
                                     enable_thinking=False) for p in prompts]
    outs = llm.generate(chats, SamplingParams(temperature=0.0, max_tokens=1400))
    return [o.outputs[0].text for o in outs], time.time() - t

@app.local_entrypoint()
def main(inp: str, out: str, chunk: int = 1200):
    with open(inp) as fh: items = [json.loads(l) for l in fh]
    done = set()
    if Path(out).exists():
        with open(out) as fh: done = {json.loads(l)["key"] for l in fh if l.strip()}
    todo = [i for i in items if i["key"] not in done]
    print({"total": len(items), "todo": len(todo)}, flush=True)
    chunks = [todo[i:i + chunk] for i in range(0, len(todo), chunk)]
    secs = 0.0
    with open(out, "a") as f:
        for ch, res in zip(chunks, run.map([[x["prompt"] for x in c] for c in chunks], order_outputs=True)):
            texts, s = res
            secs += s
            for x, t in zip(ch, texts):
                f.write(json.dumps({"key": x["key"], "sid": x["sid"], "date": x["date"], "out": t}) + "\n")
            f.flush()
            print({"chunk_done": len(ch), "secs": round(s), "usd": round(secs / 3600 * 3.95 * 1.15, 2)}, flush=True)
