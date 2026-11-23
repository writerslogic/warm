# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Gemini (Vertex AI, global endpoint, REST) controller loop and single-call client used by every Gemini stage.

Environment:
  LME2_VERTEX_PROJECT  Google Cloud project that serves the Gemini models
  LME2_VERTEX_TOKEN    OAuth access token; when unset, `gcloud auth print-access-token` is called every 25 minutes
"""
import json, os, time, subprocess, threading, random, urllib.request, urllib.error
from prompts import PRICES, _schema  # noqa: F401  (ctl_pass2 reads ctl_g._schema)
_tok = [None, 0]; _l = threading.Lock()
def token():
    with _l:
        if os.environ.get("LME2_VERTEX_TOKEN"):
            return os.environ["LME2_VERTEX_TOKEN"]
        if time.time() - _tok[1] > 1500:
            _tok[0] = subprocess.check_output(["gcloud", "auth", "print-access-token"], text=True, stderr=subprocess.DEVNULL).strip(); _tok[1] = time.time()
        return _tok[0]
def gen(model, body, retries=6):
    project = os.environ.get("LME2_VERTEX_PROJECT")
    if not project:
        raise RuntimeError("set LME2_VERTEX_PROJECT")
    url = f"https://aiplatform.googleapis.com/v1/projects/{project}/locations/global/publishers/google/models/{model}:generateContent"
    for k in range(retries):
        req = urllib.request.Request(url, json.dumps(body).encode(), {"Authorization": "Bearer " + token(), "Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=300) as r: return json.load(r)
        except urllib.error.HTTPError as e:
            msg = e.read().decode()[:400]
            if e.code in (429, 500, 503, 504) and k < retries - 1: time.sleep(min(60, 2 ** k + random.random())); continue
            if e.code == 401: _tok[1] = 0;
            if e.code == 401 and k < retries - 1: continue
            raise RuntimeError(f"HTTP {e.code} {msg}")
        except (urllib.error.URLError, TimeoutError) as e:
            if k < retries - 1: time.sleep(2 ** k); continue
            raise
def run(model, tools, question, *, max_steps=12, variant="v3", extra=None, max_out=16000, trunc=None, system=None, tool_decls=None, dispatch=None, final_name='submit_brief', final_check=None):
    """extra: {"thinkingLevel": "HIGH"} or {"thinkingBudget": n}. G1 always passes system, tool_decls and dispatch."""
    decls = tool_decls; dispatch = dispatch or tools.dispatch
    contents = [{"role": "user", "parts": [{"text": question}]}]
    st = dict(steps=0, tool_calls=0, ptok=0, ctok=0, cached=0, forced=False, err=None, trace=[]); t0 = time.perf_counter(); brief = None; empty = 0
    think = dict(extra or {"thinkingLevel": "MEDIUM"}); think["includeThoughts"] = False
    for _ in range(max_steps * 2 + 6):
        force = st["steps"] >= max_steps or empty >= 2
        body = {"systemInstruction": {"parts": [{"text": system}]}, "contents": contents, "tools": [{"functionDeclarations": decls}],
                "generationConfig": {"maxOutputTokens": max_out, "thinkingConfig": think}}
        if force: body["toolConfig"] = {"functionCallingConfig": {"mode": "ANY", "allowedFunctionNames": [final_name]}}; st["forced"] = True
        try: r = gen(model, body)
        except Exception as e:
            st["err"] = repr(e)[:300]; break
        u = r.get("usageMetadata", {}); st["ptok"] += u.get("promptTokenCount", 0); st["ctok"] += u.get("candidatesTokenCount", 0) + u.get("thoughtsTokenCount", 0); st["steps"] += 1
        cand = (r.get("candidates") or [{}])[0]; parts = (cand.get("content") or {}).get("parts") or []
        if not parts:
            empty += 1
            if force: break
            contents.append({"role": "user", "parts": [{"text": f"Continue with a tool call or call {final_name}."}]}); continue
        contents.append({"role": "model", "parts": parts})
        calls = [p["functionCall"] for p in parts if "functionCall" in p]
        if not calls:
            empty += 1
            if force: break
            contents.append({"role": "user", "parts": [{"text": f"Call {final_name} now." if st["steps"] >= max_steps else f"Continue with a tool call or call {final_name}."}]}); continue
        empty = 0; resp = []
        for c in calls:
            a = c.get("args") or {}
            if c["name"] == final_name:
                err = final_check(a) if final_check else None
                if err and st["steps"] <= max_steps: resp.append({"functionResponse": {"name": c["name"], "response": {"result": "REJECTED: " + err}}})
                else: brief = a; resp.append({"functionResponse": {"name": c["name"], "response": {"result": "ok"}}})
            else:
                st["tool_calls"] += 1; st["trace"].append((c["name"], json.dumps(a)[:200]))
                out = dispatch(c["name"], a) if st["steps"] <= max_steps else f"Tool budget exhausted: call {final_name}."
                if trunc: out = out[:trunc]
                resp.append({"functionResponse": {"name": c["name"], "response": {"result": out}}})
        left = max_steps - st["steps"]
        note = (f"[Progress: round {st['steps']} of {max_steps} used, {left} remaining." + (f" {final_name} next with the best evidence so far.]" if left <= 2 else "]"))
        last = resp[-1]["functionResponse"]["response"]; last["result"] = str(last["result"]) + "\n" + note
        contents.append({"role": "user", "parts": resp})
        if brief is not None: break
    st["seconds"] = time.perf_counter() - t0
    pin, pout = PRICES.get(model, (0, 0)); st["cost_usd"] = st["ptok"] * pin + st["ctok"] * pout
    return brief, st


def call_once(model, body):
    """One forced function call, retried once when no call comes back (the VERIFY/CRUX/OPTCHECK/ANSWER-CHECK loop)."""
    res = None; err = None; cost = 0.0; u = {}
    pin, pout = PRICES[model]
    try:
        for _ in range(2):
            rr = gen(model, body); u = rr.get("usageMetadata", {})
            cost += u.get("promptTokenCount", 0) * pin + (u.get("candidatesTokenCount", 0) + u.get("thoughtsTokenCount", 0)) * pout
            parts = ((rr.get("candidates") or [{}])[0].get("content") or {}).get("parts") or []
            c = [p["functionCall"]["args"] for p in parts if "functionCall" in p]
            if c: res = c[0]; break
    except Exception as e: err = repr(e)[:300]
    return res, err, cost, u
