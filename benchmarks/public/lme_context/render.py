"""Context assembly arms for LongMemEval_S. The official TEMPLATE and the official rounds layout are untouched;
arms only add a memory-notes block inside the 'History Chats' slot and annotate session headers.
Zero API. Spec = dict of flags (see DEFAULT)."""
import importlib.util, json, os, re, datetime as dt, sys
from pathlib import Path
HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("qa_l01", HERE.parent / "qa_l01.py")
_argv = sys.argv; qa = importlib.util.module_from_spec(_spec); sys.argv = ["x"]; _spec.loader.exec_module(qa); sys.argv = _argv
DATA = None
def data():
    global DATA
    if DATA is None:
        DATA = {e["question_id"]: e for e in json.load(open(os.path.expanduser("~/.cache/warm-bench/downloads/longmemeval_s_cleaned.json")))}
    return DATA
RANK = None
def rankings():
    global RANK
    if RANK is None: RANK = json.load(open(HERE.parent / "out/lme500.rankings.json"))
    return RANK
DEFAULT = dict(k=30, pre="opening", facts=0, summ=0, date_ref=False, offsets=False, chain=False, pos="post",
               route=False, factscope="retrieved", maxfacts=400, drop_rounds=0)
# ---- extraction store
def parse_ext(text):
    t = text.strip()
    t = re.sub(r"^```(?:json)?|```$", "", t, flags=re.M).strip()
    try: o = json.loads(t)
    except Exception:
        m = re.search(r"\{.*\}", t, flags=re.S)
        try: o = json.loads(m.group(0)) if m else None
        except Exception: o = None
    if not isinstance(o, dict):  # salvage output truncated at max_tokens: summary plus every complete fact object
        m = re.search(r'"summary":\s*"((?:[^"\\]|\\.)*)"', t)
        fs = re.findall(r'\{\s*"date":\s*(null|"[^"]*"),\s*"slot":\s*(null|"[^"]*"),\s*"fact":\s*("(?:[^"\\]|\\.)*")\s*\}', t)
        if not m or not fs: return None
        try: o = {"summary": json.loads('"' + m.group(1) + '"'), "facts": [{"date": json.loads(d), "slot": json.loads(sl), "fact": json.loads(f)} for d, sl, f in fs]}
        except Exception: return None
    facts = []
    for f in o.get("facts") or []:
        if isinstance(f, dict) and f.get("fact"):
            facts.append({"date": f.get("date") if isinstance(f.get("date"), str) else None, "slot": f.get("slot") if isinstance(f.get("slot"), str) else None, "fact": str(f["fact"])})
    return {"summary": str(o.get("summary") or ""), "facts": facts}
class Store:
    def __init__(self, *paths):
        self.m = {}; self.bad = 0
        for p in paths:
            for l in open(p):
                r = json.loads(l); o = parse_ext(r["out"])
                if o is None: self.bad += 1; o = {"summary": "", "facts": []}
                self.m[(r["sid"], r["date"])] = o
    def get(self, sid, date): return self.m.get((sid, date), {"summary": "", "facts": []})
# ---- dates
def pdate(s):
    m = re.match(r"(\d{4})/(\d{2})/(\d{2})", s); return dt.date(int(m[1]), int(m[2]), int(m[3])) if m else None
def fmt(d): return d.strftime("%Y/%m/%d (%a)")
NUM = {"a": 1, "an": 1, "one": 1, "two": 2, "three": 3, "four": 4, "five": 5, "six": 6, "seven": 7, "eight": 8, "nine": 9, "ten": 10, "couple": 2, "few": 3}
def date_reference(q, qdate):
    today = pdate(qdate); out = [f"Current date: {fmt(today)}."]
    ql = q.lower()
    for n, unit in re.findall(r"\b(\d+|a|an|one|two|three|four|five|six|seven|eight|nine|ten)\s+(day|week|month|year)s?\s+ago", ql):
        n = int(n) if n.isdigit() else NUM[n]
        if unit == "day": d = today - dt.timedelta(days=n)
        elif unit == "week": d = today - dt.timedelta(weeks=n)
        elif unit == "month":
            mo = today.month - n; y = today.year + (mo - 1) // 12; mo = (mo - 1) % 12 + 1
            d = dt.date(y, mo, min(today.day, 28))
        else: d = dt.date(today.year - n, today.month, min(today.day, 28))
        out.append(f"{n} {unit}(s) ago = about {fmt(d)}.")
    if "yesterday" in ql: out.append(f"Yesterday = {fmt(today - dt.timedelta(days=1))}.")
    if re.search(r"last week", ql): out.append(f"Last week = {fmt(today - dt.timedelta(days=today.weekday() + 7))} to {fmt(today - dt.timedelta(days=today.weekday() + 1))}.")
    if re.search(r"last month", ql):
        first = today.replace(day=1); lm = (first - dt.timedelta(days=1)); out.append(f"Last month = {lm.strftime('%B %Y')}.")
    return " ".join(out)
# ---- routing heuristics (question text only; never the gold type or the _abs suffix)
def route(q):
    ql = q.lower()
    if re.search(r"\bhow many\b|\bin total\b|\btotal (number|amount|cost)|\bhow much (money|did|have|in total)", ql): return "count"
    if re.search(r"\bhow (long|many (days|weeks|months|years)) (ago|before|after|between|since|until)|\bago\b|\bbefore\b|\bafter\b|\bfirst\b|\border\b|\bwhich .* (first|earlier|later)|\bsince\b|\bhow many (days|weeks|months)", ql): return "temporal"
    if re.search(r"\b(current|currently|now|latest|most recent|still|these days|used to|changed)\b", ql): return "update"
    if re.search(r"\b(recommend|suggest|suggestions?|ideas?|tips|any advice)\b|can you (recommend|suggest)", ql): return "pref"
    return "other"
def retrieved_sessions(e, ranked, k):
    seen = []
    for c in ranked[:k]:
        sid = "_".join(c.replace("noans_", "answer_").split("_")[:-1])
        if sid not in seen: seen.append(sid)
    return seen
def notes_block(e, ranked, store, spec, chains=None):
    k = spec["k"]; n = spec["facts"] or spec["summ"]
    sess = list(e["haystack_session_ids"]) if spec["factscope"] == "all" else retrieved_sessions(e, ranked, max(k, 60))[: n]
    dates = dict(zip(e["haystack_session_ids"], e["haystack_dates"]))
    lines = []
    if spec["summ"]:
        lines.append("### Session summaries (extracted from the chats; oldest first)")
        for sid in sorted(sess[: spec["summ"]], key=lambda s: dates[s]):
            sm = store.get(sid, dates[sid])["summary"]
            if sm: lines.append(f"- {dates[sid][:10]}: {sm}")
    if spec["facts"]:
        fl = []
        for sid in sess[: spec["facts"]]:
            for f in store.get(sid, dates[sid])["facts"]:
                fl.append((f["date"] or dates[sid][:10].replace("/", "-"), dates[sid], f["fact"]))
        fl.sort(key=lambda x: (x[0], x[1])); fl = fl[: spec["maxfacts"]]
        lines.append("### Memory notes: dated facts extracted from the chats (oldest first; dates are absolute)")
        lines += [f"- [{d}] {t}" for d, _, t in fl]
    if chains:
        lines.append("### Updated values (the latest value is the current one)")
        lines += chains
    return "\n".join(lines)
def build(qid, store, spec=None, chains=None):
    spec = {**DEFAULT, **(spec or {})}; e = data()[qid]; ranked = rankings()[qid]
    if spec["route"]:
        r = route(e["question"])
        spec = {**spec, **{"count": dict(facts=20, summ=0), "temporal": dict(date_ref=True, offsets=True, facts=12),
                           "update": dict(facts=12, chain=True), "pref": dict(summ=12), "other": {}}[r]}
    qa.PRE_TURNS = 1 if spec["pre"] == "opening" else 0; qa.PRE_MODE = "opening"; qa.LAYOUT = "official"; qa.RULES_TEXT = ""
    prompt = qa.official_prompt(e, ranked, spec["k"])
    extras = notes_block(e, ranked, store, spec, chains if spec["chain"] else None)
    if spec["date_ref"]: extras = date_reference(e["question"], e["question_date"]) + ("\n" + extras if extras else "")
    if spec["offsets"]:
        today = pdate(e["question_date"])
        prompt = re.sub(r"Session Date: (\d{4}/\d{2}/\d{2})([^\n]*)", lambda m: f"Session Date: {m[1]}{m[2]} [{(today - pdate(m[1])).days} days before the current date]", prompt)
    if extras:
        if spec["pos"] == "pre": prompt = prompt.replace("History Chats:\n\n", "History Chats:\n\n" + extras + "\n\n", 1)
        else:
            tail = f"\n\nCurrent Date: {e['question_date']}\nQuestion:"
            i = prompt.rindex(tail); prompt = prompt[:i] + "\n\n" + extras + prompt[i:]
    return prompt
