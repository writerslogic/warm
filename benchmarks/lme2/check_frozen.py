#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Prompt-integrity check: every frozen prompt, tool schema, message template, model setting and cap in this port equals the
reference G1 files. The reference files are parsed, never imported (they run argparse and load data at import).
usage: check_frozen.py --reference-dir <dir with arch/, arch2/, arch3/, arch4/, arch5/, arch6/, warm_memory/>"""
import argparse, ast, hashlib, re, sys
from pathlib import Path
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import prompts as P  # noqa: E402

ap = argparse.ArgumentParser(); ap.add_argument("--reference-dir", required=True); a = ap.parse_args()
R = Path(a.reference_dir)
fails = []; n = [0]


def check(name, ok):
    n[0] += 1
    if not ok: fails.append(name)
    print(("ok   " if ok else "FAIL ") + name)


def tree(rel): return ast.parse((R / rel).read_text())


def assign(t, name):
    for node in ast.walk(t):
        if isinstance(node, ast.Assign) and any(isinstance(x, ast.Name) and x.id == name or isinstance(x, ast.Tuple) and name in [getattr(e, "id", None) for e in x.elts] for x in node.targets): return node.value
    raise KeyError(name)


def ev(node, **ns):
    return eval(compile(ast.Expression(body=node), "<ref>", "eval"), {"re": re, **ns})


def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
LICENSE_HEADER = b"# Copyright 2024-2026 WritersLogic Contributors\n# SPDX-License-Identifier: Apache-2.0\n"
def sha_port(p):
    """sha256 of a ported copy without the license header the repository adds to every file."""
    b = Path(p).read_bytes()
    return hashlib.sha256(b[len(LICENSE_HEADER):] if b.startswith(LICENSE_HEADER) else b).hexdigest()


def strings(node, skip=()):
    out = set()
    def walk(x):
        if any(isinstance(x, k) for k in (ast.stmt,)) and any(s in ast.unparse(x) for s in skip) and not isinstance(x, (ast.FunctionDef, ast.ClassDef)) or isinstance(x, ast.FunctionDef) and x.name in skip:
            return
        if isinstance(x, ast.Constant) and isinstance(x.value, str): out.add(x.value)
        for c in ast.iter_child_nodes(x): walk(c)
    walk(node)
    return out


PORT_STRINGS = set()
for f in HERE.glob("*.py"):
    if f.name != "check_frozen.py": PORT_STRINGS |= strings(ast.parse(f.read_text()))

# verbatim modules
check("envmodel.py byte-identical to arch/envmodel.py apart from the license header", sha_port(HERE / "envmodel.py") == sha(R / "arch/envmodel.py"))
check("formstate.py byte-identical to warm_memory/formstate.py apart from the license header", sha_port(HERE / "formstate.py") == sha(R / "warm_memory/formstate.py"))
check("held_ids.json byte-identical to arch3/held_ids.json", sha(HERE / "held_ids.json") == sha(R / "arch3/held_ids.json"))

# named prompts and schemas
ctl = SimpleNamespace(_schema=P._schema)
check("_schema", ast.dump(next(x for x in tree("arch3/ctl_g.py").body if isinstance(x, ast.FunctionDef) and x.name == "_schema")) == ast.dump(next(x for x in ast.parse((HERE / "prompts.py").read_text()).body if isinstance(x, ast.FunctionDef) and x.name == "_schema")))
check("PRICES", ev(assign(tree("arch3/ctl_g.py"), "PRICES")) == P.PRICES)
check("WORKFLOW (workflow_doc4)", ev(assign(tree("arch3/workflow_doc4.py"), "WORKFLOW")) == P.WORKFLOW)
check("TOOL_SPECS (arch2/tools.py)", ev(assign(tree("arch2/tools.py"), "TOOL_SPECS")) == P.TOOL_SPECS)
t3 = tree("arch3/tools3.py"); ns3 = {}
for x in t3.body:
    if isinstance(x, ast.FunctionDef) and x.name == "_p" or isinstance(x, ast.Assign) and ast.unparse(x.targets[0]) == "S": exec(compile(ast.Module(body=[x], type_ignores=[]), "<ref>", "exec"), ns3)
check("MAX_SPAN_STATES", ev(assign(t3, "MAX_SPAN_STATES")) == P.MAX_SPAN_STATES == 20)
check("EXTRA_SPECS (find_states, show_state, trajectory_goal)", ev(assign(t3, "EXTRA_SPECS"), **ns3) == P.EXTRA_SPECS)
check("SUBMIT_SPANS", ev(assign(t3, "SUBMIT_SPANS"), **ns3) == P.SUBMIT_SPANS)
STAGES = {"VERIFY": "arch4/verify_stage.py", "CRUX": "arch4/crux_stage.py", "OPTCHECK": "arch5/optcheck_stage.py", "ANSCHECK": "arch6/anscheck_stage.py"}
for tag, rel in STAGES.items():
    t = tree(rel)
    check(f"{tag}_SYSTEM", ev(assign(t, "SYSTEM")) == getattr(P, f"{tag}_SYSTEM"))
    check(f"{tag}_DECL", ev(assign(t, "DECL"), ctl_g=ctl) == getattr(P, f"{tag}_DECL"))
    check(f"{tag} PER, TOT", ev(assign(t, "PER")) == (P.PER, P.TOT) == (24000, 140000) if isinstance(assign(t, "PER"), ast.Tuple) else False)
    name = "verify" if tag == "VERIFY" else "cite"
    check(f"{tag} request body", ev(assign(t, "body"), SYSTEM="<SYS>", msg="<MSG>", DECL="<DECL>") == P.stage_body("<SYS>", "<MSG>", "<DECL>", name))
    r = {"support": "<SUP>", "analysis": "<AN>"}
    ref_msg = ev(assign(t, "msg"), Q={"i": {"question": "<Q>"}}, i="i", q="<Q>", r=r, render=lambda _: "<STATES>", ANS={"i": "<ANS>"})
    port_msg = P.verify_message("<Q>", r, "<STATES>") if tag == "VERIFY" else P.anscheck_message("<Q>", "<ANS>", "<STATES>") if tag == "ANSCHECK" else P.states_message("<Q>", "<STATES>")
    check(f"{tag} user message", ref_msg == port_msg)
    check(f"{tag} default model gemini-3.6-flash", 'default="gemini-3.6-flash"' in (R / rel).read_text() and 'default="gemini-3.6-flash"' in (HERE / "check_stages.py").read_text())
    for fn in ("render", "ground", "state_text"):
        refd = [x for x in t.body if isinstance(x, ast.FunctionDef) and x.name == fn]
        if refd:
            portd = next(x for x in ast.parse((HERE / "common.py").read_text()).body if isinstance(x, ast.FunctionDef) and x.name == fn)
            check(f"{tag} {fn}() identical", ast.dump(refd[0]) == ast.dump(portd))
    if "norm" in {getattr(x.targets[0], "id", None) for x in t.body if isinstance(x, ast.Assign)}:
        check(f"{tag} norm identical", ast.dump(assign(t, "norm")) == ast.dump(assign(ast.parse((HERE / "common.py").read_text()), "norm")))
check("CHOICE_PATTERN", (lambda p: (p.pattern, p.flags))(ev(assign(tree("arch6/choice_ids.py"), "pat"))) == (P.CHOICE_PATTERN.pattern, P.CHOICE_PATTERN.flags))

# renderer
sp = tree("warm_memory/arch3_span.py")
vd = next(x for x in ast.walk(sp) if isinstance(x, ast.Dict) and any(isinstance(k, ast.Constant) and k.value == "supported" for k in x.keys))
check("SUPPORT_VERDICT", ev(vd) == P.SUPPORT_VERDICT)
pre = next(x for x in ast.walk(sp) if isinstance(x, ast.Assign) and ast.unparse(x.targets[0]) == "pre" and "Form controls" in ast.unparse(x.value)).value
for new, rows in ((["a", "b"], ["a", "b"]), (["b"], ["a", "b"]), ([], ["a", "b"])):
    check(f"form block new={new} rows={rows}", ev(pre, new=new, rows=rows) == P.form_block(new, len(rows)))
src = (R / "warm_memory/arch3_span.py").read_text()
check("budget estimate 0.42 chars->tokens and cut text", "est = 0.42 * (len(body) + len(pre))" in src and P.TOKENS_PER_CHAR == 0.42 and repr(P.BUDGET_CUT)[1:-1] in src)
held = (R / "arch6/judge6.sh").read_text()
check("frozen judge run: ARCH5_FORM=table ARCH5_BUDGET=140000 ARCH6_FIRST", "ARCH5_FORM=table" in held and f"ARCH5_BUDGET={P.FORM_TOKEN_BUDGET}" in held and "ARCH6_FIRST=" in held)
check("renderer literals present in port", not (strings(next(x for x in ast.walk(sp) if isinstance(x, ast.FunctionDef) and x.name == "query"),
      skip=("narrow", "sdiff", "ARCH5_EXTRA", "ARCH6_ABSTAIL", "ARCH7_DIFF", "terms =", "ARCH3_SPANS", "ARCH6_FIRST", "ARCH5_FORM", "ARCH5_BUDGET", "formstate", "_nb", "hasattr")) - PORT_STRINGS))

# controller
c = tree("arch3/ctl_g.py"); run_ref = next(x for x in c.body if isinstance(x, ast.FunctionDef) and x.name == "run")
run_port = next(x for x in ast.parse((HERE / "ctl_g.py").read_text()).body if isinstance(x, ast.FunctionDef) and x.name == "run")
check("controller loop run(): body identical after the system/decls defaults", [ast.dump(s) for s in run_ref.body[3:]] == [ast.dump(s) for s in run_port.body[2:]] and ast.dump(run_ref.args) == ast.dump(run_port.args))
check("gen(): retry and request logic literals present", not (strings(next(x for x in c.body if isinstance(x, ast.FunctionDef) and x.name == "gen")) - PORT_STRINGS))
for cls in ("Model2", "Tools"):
    check(f"arch2/tools.py class {cls} identical", ast.dump(next(x for x in tree("arch2/tools.py").body if isinstance(x, ast.ClassDef) and x.name == cls)) == ast.dump(next(x for x in ast.parse((HERE / "tools2.py").read_text()).body if isinstance(x, ast.ClassDef) and x.name == cls)))
check("tools3.py class Tools3 identical", ast.dump(next(x for x in t3.body if isinstance(x, ast.ClassDef) and x.name == "Tools3")) == ast.dump(next(x for x in ast.parse((HERE / "tools3.py").read_text()).body if isinstance(x, ast.ClassDef) and x.name == "Tools3")))
chain = (R / "arch3/chain_held.sh").read_text()
check("pass-1 controller settings (arch3/chain_held.sh)", all(s in chain for s in ("WFDOC=workflow_doc4", f"--model {P.CONTROLLER_MODEL}", f"--max-steps {P.CONTROLLER_MAX_STEPS}", '{"thinkingLevel":"MEDIUM"}')) and P.CONTROLLER_EXTRA == {"thinkingLevel": "MEDIUM"})
p2 = tree("arch6/ctl_pass2.py")
call = next(x for x in ast.walk(p2) if isinstance(x, ast.Call) and ast.unparse(x.func) == "ctl_g.run")
kw = {k.arg: k.value for k in call.keywords}
check("pass-2 controller settings", ev(call.args[0]) == P.CONTROLLER_MODEL and ev(kw["max_steps"]) == P.CONTROLLER_MAX_STEPS and ev(kw["extra"]) == P.CONTROLLER_EXTRA
      and ev(kw["system"], WORKFLOW=P.WORKFLOW) == P.workflow(P.CONTROLLER_MAX_STEPS))
check("pass-2 memory note", ev(assign(p2, "qq"), q={"question": "<Q>"}, opts="<O1>; <O2>") == P.pass2_question("<Q>", "<O1>; <O2>"))
check("pass-2 no-option text", P.PASS2_NO_OPTIONS in strings(assign(p2, "opts")))
check("pass-2 merge cap 20 states", "n + k <= 20" in (R / "arch6/ctl_pass2.py").read_text() and "n + k <= 20" in (HERE / "rules.py").read_text())

# reread rows
mk = tree("arch6/mkp6.py")
ec = next(x for x in ast.walk(mk) if isinstance(x, ast.BinOp) and "Evidence Check" in ast.unparse(x))
g = [{"trajectory_id": "t1", "state_index": 3, "text": 'a "b"'}, {"trajectory_id": "t2", "state_index": 0, "text": "c"}]
check("Evidence Check text", ev(ec, a={"grounded": g}) == P.evidence_check(g))

# reader and judge settings
rs = (R / "warm_memory/reader_stage.py").read_text(); prs = (HERE / "reader_stage.py").read_text()
for s in ('model="Qwen/Qwen3.5-9B"', "max_completion_tokens=20000", "timeout_seconds=43200.0", "temperature=0.6", "top_p=0.95", "top_k=20", "reader_enable_thinking=True", "reasoning_effort=None", "presence_penalty=None", "repetition_penalty=None"):
    check(f"reader {s}", s in rs and s in prs)
rl = tree("warm_memory/run_lme2.py"); prl = ast.parse((HERE / "run_lme2.py").read_text())
argv_ref = [x.value for x in assign(rl, "argv").elts if isinstance(x, ast.Constant)]
argv_port = [x.value for x in assign(prl, "argv").elts if isinstance(x, ast.Constant)]
check("harness argv constants (reader sampling, context cap, judge gpt-5.2 medium 4096)", argv_ref[:argv_ref.index("--api-key-env") + 1] + ["READER_API_KEY"] + argv_ref[argv_ref.index("--api-key-env") + 2:] == argv_port)
check("judge patch literals present", not (strings(next(x for x in rl.body if isinstance(x, ast.FunctionDef) and x.name == "install_judge_patch"), skip=("collect",)) - PORT_STRINGS))
serve = (R / "arch5/serve_p4.sh").read_text(); pserve = (HERE / "serve_reader.sh").read_text()
flags = re.findall(r"(--[\w-]+(?: (?!--)\S+)?)", serve[serve.index("vllm serve"):])
check("vLLM serve flags (arch5/serve_p4.sh)", all(f in pserve for f in flags if not f.startswith("--port")) and "VLLM_USE_FLASHINFER_SAMPLER=0" in pserve)

print(f"\n{n[0] - len(fails)}/{n[0]} checks passed")
sys.exit(1 if fails else 0)
