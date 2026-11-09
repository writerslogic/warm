"""Build extraction prompts for unique (session id, date) pairs. usage: prep_extract.py dev|heldout|all out.jsonl"""
import json, hashlib, os, sys
scope, out = sys.argv[1], sys.argv[2]
with open(os.path.expanduser("~/.cache/warm-bench/downloads/longmemeval_s_cleaned.json")) as fh: D = json.load(fh)
with open("../frozen/longmemeval_split.json") as fh: split = json.load(fh)
want = {"dev": set(split["dev"]), "heldout": set(split["heldout"]), "all": {e["question_id"] for e in D}}[scope]
PROMPT = """You are building the long-term memory of a user from their chat history with an AI assistant. Below is ONE chat session held on {date}.

Write a JSON object with exactly these keys:
"summary": two sentences on what the user discussed, decided or asked.
"facts": a list of atomic, self-contained statements, each {{"date": ..., "slot": ..., "fact": ...}}. Cover the user's life, preferences, plans, purchases, events, quantities, names, places and current states, plus any specific thing the assistant told or recommended that the user could later ask about (say "the assistant ..." for those). "date" is the absolute date (YYYY-MM-DD) of the event or state: resolve relative expressions ("yesterday", "last Saturday", "two weeks ago", "next month") against the session date above; use the session date for things happening now; null only if no date applies. Keep numbers, names and units exact. If something replaces an earlier state (moved, switched, upgraded, now), state both the new value and that it is new. "slot" names the single-valued attribute the fact sets, as a lower_snake path such as user/city, user/job_title, user/car, user/gym_visits_per_week, pet_dog/weight (an attribute that a later fact could overwrite); use null for one-off events and for assistant statements. At most 25 facts.
Answer with the JSON object only.

SESSION ({date}):
{text}"""
def render(session):
    parts = []
    for t in session:
        c = t["content"]
        if len(c) > 3500: c = c[:3500] + " ..."
        parts.append(f'{t["role"]}: {c}')
    s = "\n".join(parts)
    return s[:26000]
seen = {}
for e in D:
    if e["question_id"] not in want: continue
    for sid, date, s in zip(e["haystack_session_ids"], e["haystack_dates"], e["haystack_sessions"]):
        text = render(s)
        key = hashlib.sha256(f"{sid}|{date}|{text}".encode()).hexdigest()[:24]
        if key not in seen:
            seen[key] = {"key": key, "sid": sid, "date": date, "prompt": PROMPT.format(date=date, text=text)}
with open(out, "w") as f:
    for v in seen.values(): f.write(json.dumps(v) + "\n")
print(scope, len(seen), "sessions", sum(len(v["prompt"]) for v in seen.values()) // 4, "approx tokens")
