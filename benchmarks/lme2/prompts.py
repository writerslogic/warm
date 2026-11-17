# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Frozen G1 prompt strings, tool schemas and constants, extracted from the reference files; check_frozen.py asserts equality."""
import re

def _schema(s):
    if isinstance(s, dict):
        return {("type" if k == "type" else k): (v.upper() if k == "type" and isinstance(v, str) else _schema(v)) for k, v in s.items() if k not in ("additionalProperties",)}
    if isinstance(s, list): return [_schema(x) for x in s]
    return s
PRICES = {"gemini-3.8-flash": (0.75e-6, 3.75e-6), "gemini-3.6-flash": (0.75e-6, 3.75e-6), "gemini-3-flash-preview": (0.5e-6, 3.0e-6), "gemini-3.5-flash": (0.5e-6, 3.0e-6),
          "gemini-3.5-flash-lite": (0.3e-6, 2.5e-6), "gemini-3.1-flash-lite": (0.25e-6, 1.5e-6), "gemini-3.7-flash": (0.75e-6, 3.75e-6)}

WORKFLOW = """You are the memory module of an assistant that answers questions about a customized web application. The application was recorded in many sessions (trajectories) of agents using it; each trajectory is a list of states (page snapshot + the action taken at that state). You are NOT the answering model: you choose which raw states the answering model will read. It reads only the states you select (page text and screenshots) plus your analysis, with no tools, so the answer must be visible in them.

Your tools: a compiled index of the memory (regions: deduplicated page fragments; transitions: what each action did to a page; episodes: one line per step of each trajectory with the state index; pages: full element inventories; notes: session summaries; value_history) and raw-state tools (find_states: search every state; show_state: preview a state; trajectory_goal). Finish with exactly one submit_spans call.

Workflow:
1. Plan in two lines: what the answer needs (which page, element, option, value, ordering, count, or steps) and which kind of evidence shows it.
2. Round 1: issue 3 to 5 independent calls in parallel with different kinds and wordings (pages / regions / episodes / find_states) using the concrete labels in the question.
3. Locate states: episodes list "state_index. action" per trajectory, so an episode tells you which state index shows the page after each step; find_states returns state indices directly. Use show_state to confirm a state really contains the item before selecting it.
4. Choose spans: the smallest ranges that show the evidence. For a procedure ('how do I...'), select the contiguous states of the best successful trajectory, from the start page to the state that completes it (the reader counts steps from them), and add the states of a failed or ineffective attempt only if the question is about problems. For a value or option question, select the state where the element is visible (and the state before/after if the question is about a change). For a question that may assume something that does not exist, select the state(s) of the relevant page that is fully shown but lacks the element, and say so in the analysis. Prefer distinct trajectories over near-duplicates. Put the most important span first. Total at most 20 states; count inclusively.
5. Stop searching when every item the question asks about is covered by a selected state; do not keep confirming. You work in at most {N} rounds, with several tool calls per round; a progress note tells you the rounds left; when two remain, call submit_spans.
6. Give an honest support verdict in submit_spans: supported / partial / absent. Use absent when you searched the relevant pages and they do not contain what the question asks for (a feature, option, setting, column, value or step that is not there, or a question whose premise the pages contradict); the downstream model will then tell the user it does not exist instead of guessing. Use supported only when a selected state visibly shows the asked fact. Do not choose absent merely because the search was hard: if the item may exist on a page you have not opened, keep searching or say partial.
7. Do not answer from general knowledge; every claim in the analysis must be visible in a selected state. Plain text, no LaTeX."""

TOOL_SPECS = [
 {"type": "function", "function": {"name": "search", "description": "Lexical search over one kind of compiled memory item. kind: regions (deduplicated page fragments), transitions (what an action did to a page), episodes (recorded task attempts with action paths), pages (page templates with element inventories), notes (summaries of what recorded sessions did). Returns item ids with a snippet.",
   "parameters": {"type": "object", "properties": {"kind": {"type": "string", "enum": ["regions", "transitions", "episodes", "pages", "notes"]}, "query": {"type": "string"}, "k": {"type": "integer"}}, "required": ["kind", "query"]}}},
 {"type": "function", "function": {"name": "open", "description": "Full text of an item by id (R.. region, T.. transition, E:<traj> episode, P.. page inventory listing every element observed on that page with observation counts, N:<traj> notes).",
   "parameters": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]}}},
 {"type": "function", "function": {"name": "transitions_from", "description": "All recorded actions taken on a page template (P.. id) with their observed effects.",
   "parameters": {"type": "object", "properties": {"page_id": {"type": "string"}}, "required": ["page_id"]}}},
 {"type": "function", "function": {"name": "value_history", "description": "How the value/state of page elements matching the query changed across steps of each recorded session.",
   "parameters": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}}},
 {"type": "function", "function": {"name": "submit_brief", "description": "Finish: hand the downstream reader a support brief.",
   "parameters": {"type": "object", "properties": {
     "answer": {"type": "string", "description": "Best answer to the question, in exactly the form the question asks for (short phrases, a letter, a number, or a statement). Empty only if nothing in memory bears on it."},
     "evidence": {"type": "array", "items": {"type": "object", "properties": {"id": {"type": "string"}, "quote": {"type": "string"}}, "required": ["id", "quote"]}, "description": "Exact quotes from memory items supporting the answer, with their item ids."},
     "premise": {"type": "string", "enum": ["ok", "false", "unsure"], "description": "false only if memory positively shows something the question presupposes does not exist or does not hold (for instance a complete page inventory lacking the element); unsure if memory does not settle it."},
     "correction": {"type": "string", "description": "If premise is false: what the question assumes, and what is true instead. Otherwise empty."}},
    "required": ["answer", "evidence", "premise"]}}},
]

MAX_SPAN_STATES = 20
def _p(d, req): return {"type": "object", "properties": d, "required": req}
S = lambda t: {"type": t}
EXTRA_SPECS = [
 {"type": "function", "function": {"name": "find_states", "description": "Lexical search over every recorded STATE (raw page snapshots) of all trajectories. Returns trajectory id, state index, url, action. Use it to locate where text, labels or values appear.", "parameters": _p({"query": S("string"), "k": S("integer")}, ["query"])}},
 {"type": "function", "function": {"name": "show_state", "description": "Preview one raw state (url, action, first 2500 chars of its page text) to check it contains what you need before selecting it.", "parameters": _p({"trajectory_id": S("string"), "state_index": S("integer"), "max_chars": S("integer")}, ["trajectory_id", "state_index"])}},
 {"type": "function", "function": {"name": "trajectory_goal", "description": "Goal, outcome and state count of one trajectory.", "parameters": _p({"trajectory_id": S("string")}, ["trajectory_id"])}},
]
SUBMIT_SPANS = {"type": "function", "function": {"name": "submit_spans", "description": "Finish. Select raw trajectory spans for the downstream reader (it sees those states, their page text and screenshots, and your analysis, but cannot call tools).",
  "parameters": _p({
    "analysis": {"type": "string", "description": "Short support analysis: the answer you believe the evidence supports (in the form the question asks), which span shows what, and any step that failed or had no effect. If the question presupposes something the states show is absent, say what is true instead."},
    "support": {"type": "string", "enum": ["supported", "partial", "absent"], "description": "Does the evidence in the states you selected establish what the question asks? supported: the asked fact is visible in the selected states. partial: only some of the asked items are visible. absent: after searching, the application/pages do not contain the asked feature, option, setting, value or step (including when the question assumes something the pages do not have), so no selected state can show it."},
    "spans": {"type": "array", "description": "Most important first. Zero-based inclusive state ranges; total states across spans at most 20.", "items": _p({"trajectory_id": S("string"), "start_state_index": S("integer"), "end_state_index": S("integer")}, ["trajectory_id", "start_state_index", "end_state_index"])}},
    ["analysis", "support", "spans"])}}

VERIFY_SYSTEM = """You verify one memory-retrieval step for a question about a customized web application. A retrieval module selected states (page snapshots with the action taken at each) from recorded sessions and wrote a draft analysis. You see the question, the draft analysis, and the full text of the selected states. Work only from the state text; never use general knowledge about what such an application usually contains. If the draft analysis conflicts with the state text, the state text wins.
Do this:
1. Presuppositions: list every page, control, option, value, ordering, count or relationship the question presupposes or asks about. For each, give an exact short quote from a state text that establishes it, or "none", and a status: shown (quote establishes it), not_shown (not in the selected states), or contradicted (the states show something incompatible, or show the relevant page fully and the presupposed item is not on it).
2. Facts: for the asked item read the exact label, value, selected option and position from the state text. For a default or initial value use the earliest selected state where the control appears, before later actions changed it, unless the question is about a change.
3. premise: holds when every presupposition is shown; violated when at least one is contradicted; unclear when the needed page is not among the states.
4. support: supported when the asked fact is visible in the states, partial when only part is, absent when the application pages shown do not contain what the question asks for.
5. analysis: two to five sentences in plain text giving the answer in the form the question asks and which state shows it (trajectory id, state index), with exact values. If premise is violated, say that the asked item does not exist or is not available and state what is there instead.
Call verify exactly once."""
VERIFY_DECL = [{"name": "verify", "description": "Report the verification.", "parameters": _schema({"type": "object", "properties": {
    "presuppositions": {"type": "array", "items": {"type": "object", "properties": {"claim": {"type": "string"}, "quote": {"type": "string"}, "status": {"type": "string", "enum": ["shown", "not_shown", "contradicted"]}}, "required": ["claim", "quote", "status"]}},
    "premise": {"type": "string", "enum": ["holds", "violated", "unclear"]}, "support": {"type": "string", "enum": ["supported", "partial", "absent"]}, "analysis": {"type": "string"}},
    "required": ["presuppositions", "premise", "support", "analysis"]})}]

CRUX_SYSTEM = """You check one question about a customized web application for a false premise, using only the text of states (page snapshots with the action taken at each) selected from recorded sessions. Never use general knowledge about what such an application usually contains.
Some questions are traps: they ask to name, count or give the value of something whose existence they assume (an extra stage, a second button, an option, a field, a change, a shared element, a step) but which the application does not have. Do this:
1. target: the specific thing the question asks you to name, count, compare or give a value for, including every condition the question attaches to it (position, page, which forms share it, what triggers it).
2. evidence: one to four exact, verbatim substrings of the state text that show that target exists with those conditions (copy characters exactly, no paraphrase). Each gives trajectory id and state index. Do not quote text that only shows the surrounding page.
3. verdict: exists when the evidence shows the target with all its conditions; missing when the relevant page or result is shown and the target or one of its conditions is not there or is contradicted; unclear when the states do not show the relevant page or result. A plain question with no presupposed item (a yes/no or true/false check, a procedure that the states show, a value that the states show) is exists.
4. note: one or two sentences: what is there instead when missing.
Call cite exactly once."""
CRUX_DECL = [{"name": "cite", "description": "Report the premise check.", "parameters": _schema({"type": "object", "properties": {
    "target": {"type": "string"}, "quotes": {"type": "array", "items": {"type": "object", "properties": {"trajectory_id": {"type": "string"}, "state_index": {"type": "integer"}, "text": {"type": "string"}}, "required": ["trajectory_id", "state_index", "text"]}},
    "verdict": {"type": "string", "enum": ["exists", "missing", "unclear"]}, "note": {"type": "string"}}, "required": ["target", "quotes", "verdict", "note"]})}]

OPTCHECK_SYSTEM = """You check the answer options of one question about a customized web application against the text of states (page snapshots with the action taken at each) selected from recorded sessions. Work only from the state text; never use general knowledge about what such an application usually contains.
1. options: list every answer option the question offers (for a true/false or yes/no question the options are the two values; for a question with listed choices, each choice; for a question that asks for listed items, each candidate item the question names).
2. For each option give verdict supported (the state text shows it is the right answer), refuted (the state text shows it is wrong), or not_shown, and one to three exact, verbatim substrings of the state text with trajectory id and state index that decide it. Copy characters exactly; no paraphrase.
3. Judge true/false statements in full: every part of the statement must hold for true; one counterexample in the states makes it false.
Call cite exactly once."""
OPTCHECK_DECL = [{"name": "cite", "description": "Report the option check.", "parameters": _schema({"type": "object", "properties": {
    "options": {"type": "array", "items": {"type": "object", "properties": {"option": {"type": "string"}, "verdict": {"type": "string", "enum": ["supported", "refuted", "not_shown"]},
      "quotes": {"type": "array", "items": {"type": "object", "properties": {"trajectory_id": {"type": "string"}, "state_index": {"type": "integer"}, "text": {"type": "string"}}, "required": ["trajectory_id", "state_index", "text"]}}}, "required": ["option", "verdict", "quotes"]}}}, "required": ["options"]})}]

ANSCHECK_SYSTEM = """You check a draft answer to one question about a customized web application against the text of states (page snapshots with the action taken at each) selected from recorded sessions. Work only from the state text; never use general knowledge.
verdict: contradicted when some state text shows the draft answer is wrong (a different value, label, option, order, count or step is shown for exactly what the question asks); consistent when the state text supports it; unverifiable otherwise. Do not mark contradicted for wording differences or for details the question does not ask about.
quotes: when contradicted, one to four exact, verbatim substrings of the state text (with trajectory id and state index) that show the correct fact. Copy characters exactly; no paraphrase.
Call cite exactly once."""
ANSCHECK_DECL = [{"name": "cite", "description": "Report the answer check.", "parameters": _schema({"type": "object", "properties": {
    "verdict": {"type": "string", "enum": ["consistent", "contradicted", "unverifiable"]}, "quotes": {"type": "array", "items": {"type": "object", "properties": {"trajectory_id": {"type": "string"}, "state_index": {"type": "integer"}, "text": {"type": "string"}}, "required": ["trajectory_id", "state_index", "text"]}}}, "required": ["verdict", "quotes"]})}]

CHOICE_PATTERN = re.compile(r"true or false|yes or no|which of the following|\([a-d]\)|^\s*[a-d][\).]\s|options?\s*:|choices?\s*:|choose (one|from|among)",re.I|re.M)

SUPPORT_VERDICT = {"supported": "SUPPORTED: the selected states show the asked fact.", "partial": "PARTIAL: the selected states show only part of what is asked.", "absent": "ABSENT: a search of the memory found no page or state that contains what the question asks for. If the question assumes a feature, option, setting, value or step that is not there, say that it does not exist or is not available in this application, and state what is there instead. Do not guess."}

CONTROLLER_MODEL = "gemini-3.6-flash"
CONTROLLER_MAX_STEPS = 14
CONTROLLER_EXTRA = {"thinkingLevel": "MEDIUM"}
PER, TOT = 24000, 140000
FORM_TOKEN_BUDGET = 140000
TOKENS_PER_CHAR = 0.42


def workflow(max_steps):
    return WORKFLOW.replace("{N}", str(max_steps))


def stage_body(SYSTEM, msg, DECL, name):
    return {"systemInstruction": {"parts": [{"text": SYSTEM}]}, "contents": [{"role": "user", "parts": [{"text": msg}]}], "tools": [{"functionDeclarations": DECL}],
            "toolConfig": {"functionCallingConfig": {"mode": "ANY", "allowedFunctionNames": [name]}}, "generationConfig": {"maxOutputTokens": 8000, "thinkingConfig": {"thinkingLevel": "MEDIUM", "includeThoughts": False}}}


def verify_message(q, r, states):
    return f"QUESTION:\n{q}\n\nDRAFT ANALYSIS (retrieval module, support verdict {r['support']}):\n{r['analysis']}\n\nSELECTED STATES:\n{states}"


def states_message(q, states):
    return f"QUESTION:\n{q}\n\nSELECTED STATES:\n{states}"


def anscheck_message(q, answer, states):
    return f"QUESTION:\n{q}\n\nDRAFT ANSWER:\n{answer}\n\nSELECTED STATES:\n{states}"


PASS2_NO_OPTIONS = "the asked items"


def pass2_question(question, opts):
    return question + f"\n\n[Memory note: a first search selected states that did not decide between these options: {opts}. Find and select the states that directly show the facts each option depends on.]"


def evidence_check(grounded):
    return '## Evidence Check\nThese verbatim lines from the selected states bear directly on the question; check your answer against them first:\n' + '\n'.join(f'- {q["trajectory_id"]} state {q["state_index"]}: "{q["text"]}"' for q in grounded)


FORM_HEADER = "- Form controls (structured from the page text; * = selected option):\n"


def form_block(new, n_rows):
    return FORM_HEADER + "\n".join("  " + r for r in new) + (f"\n  [{n_rows - len(new)} controls unchanged from an earlier state omitted]" if len(new) < n_rows else "") + "\n"


BUDGET_CUT = "\n  [... page text cut at the context budget]"
