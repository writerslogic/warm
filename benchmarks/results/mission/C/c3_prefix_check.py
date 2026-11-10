# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import argparse
import ast
import hashlib
import importlib.metadata
import importlib.util
import json
from pathlib import Path
import sys


def require(condition, message):
    if not condition:
        raise ValueError(message)


def rejected(call, message=None):
    try:
        call()
    except ValueError as error:
        require(message is None or str(error) == message, "wrong rejection reason")
    else:
        raise ValueError("invalid input accepted")


def run(counter):
    spec = importlib.util.spec_from_file_location("mission_c3_counter_under_check", counter)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    checks = []

    def check(name, action):
        try:
            action()
            checks.append({"name": name, "passed": True})
        except Exception as error:
            checks.append({"name": name, "passed": False, "error_type": type(error).__name__, "error": str(error)})

    def first_only():
        prefix = b'  [ {"text":"first"},THIS_IS_INVALID_JSON'
        value, receipt = module.first_record(prefix)
        require(value == {"text": "first"}, "first record selection changed")
        raw = b'{"text":"first"}'
        require(prefix[receipt["begin_byte_inclusive"]:receipt["end_byte_exclusive"]] == raw,
                "record byte extent changed")
        require(receipt["ordinal"] == 0 and receipt["sha256"] == hashlib.sha256(raw).hexdigest(), "record identity changed")

    def utf8_boundary():
        opening = b'[{"text":"'
        padding = b'x' * (1024 * 1024 - 1 - len(opening))
        raw_record = b'{"text":"' + padding + "💡".encode("utf-8") + b'"}'
        value, receipt = module.first_record(b'[' + raw_record + b',INVALID_LATER_RECORD')
        require(value["text"] == padding.decode() + "💡", "UTF8 split lost source bytes")
        require(receipt["begin_byte_inclusive"] == 1 and receipt["end_byte_exclusive"] == len(raw_record) + 1
                and receipt["sha256"] == hashlib.sha256(raw_record).hexdigest(), "UTF8 byte extent differs")

    def conservation():
        entry = {"question_id": "synthetic", "question_type": "unused", "question": "synthetic question",
                 "answer": "GOLD_MUST_NOT_RENDER", "question_date": "2026/01/03", "answer_session_ids": ["same_alias"],
                 "haystack_session_ids": ["same_alias", "same_alias"],
                 "haystack_dates": ["2026/01/02", "2026/01/01"],
                 "haystack_sessions": [[{"content": "Later λ", "role": "user", "has_answer": True},
                                        {"role": "assistant", "content": "Reply two"}],
                                       [{"role": "user", "content": "Earlier", "has_answer": False},
                                        {"role": "assistant", "content": "Reply one"}]]}
        original = json.dumps(entry)
        copied, coverage = module.clean_entry(entry)
        expected = [(1, "2026/01/01", [{"role": "user", "content": "Earlier"}, {"role": "assistant", "content": "Reply one"}]),
                    (0, "2026/01/02", [{"content": "Later λ", "role": "user"}, {"role": "assistant", "content": "Reply two"}])]
        require(copied == expected and json.dumps(entry) == original, "copy changed identity, content, date or input")
        require(list(copied[1][2][0]) == ["content", "role"], "original JSON field order changed")
        require(coverage["sessions"] == coverage["serialized_sessions"] == 2
                and coverage["turns"] == coverage["serialized_turns"] == 4
                and coverage["source_session_id_duplicates_preserved"] == 1
                and coverage["physical_coverage"] == 1.0 and coverage["identity_date_role_text_losses"] == 0, "physical conservation receipt differs")
        rendered = json.dumps(copied)
        require(all(field not in rendered for field in ("has_answer", "GOLD_MUST_NOT_RENDER", "same_alias")), "excluded metadata reached copied source")
        entry["haystack_dates"] = ["2026/01/01", "2026/01/01"]
        tied, _ = module.clean_entry(entry)
        require([row[0] for row in tied] == [0, 1], "equal-date sort is not stable")

    def range_type():
        provenance = {"experiment": "E42", "decision": "accept-pinned-counting-inputs", "revision": module.REVISION,
                      "requested_range": {"start": False, "end": module.PREFIX_CAP - 1}}
        rejected(lambda: module.validate_provenance(provenance, b""), "invalid range endpoint type")

    tokenizer = {}

    def installed_definition():
        distribution = importlib.metadata.distribution("tiktoken")
        require(distribution.version == "0.14.0", "requires installed tiktoken0.14.0")
        path = Path(distribution.locate_file("tiktoken_ext/openai_public.py"))
        raw = path.read_bytes()
        tree = ast.parse(raw)
        names = {node.targets[0].id: ast.literal_eval(node.value) for node in tree.body
                 if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name)
                 and isinstance(node.value, ast.Constant)}
        function = next(node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name == "o200k_base")
        assignments = {node.targets[0].id: node.value for node in function.body
                       if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name)}
        pattern = assignments["pat_str"]
        require(isinstance(pattern, ast.Call) and isinstance(pattern.func, ast.Attribute) and pattern.func.attr == "join"
                and ast.literal_eval(pattern.func.value) == "|" and len(pattern.args) == 1 and not pattern.keywords, "unexpected tokenizer pattern expression")
        require("|".join(ast.literal_eval(pattern.args[0])) == module.PATTERN, "copied tokenizer pattern mismatch")
        special = assignments["special_tokens"]
        require(isinstance(special, ast.Dict), "unexpected special-token expression")
        resolved = {names[key.id] if isinstance(key, ast.Name) else ast.literal_eval(key): ast.literal_eval(value)
                    for key, value in zip(special.keys, special.values, strict=True)}
        require(resolved == module.SPECIAL, "copied special-token mapping mismatch")
        loader = assignments["mergeable_ranks"]
        require(isinstance(loader, ast.Call) and isinstance(loader.func, ast.Name) and loader.func.id == "load_tiktoken_bpe", "unexpected BPE loader expression")
        require(next(ast.literal_eval(kw.value) for kw in loader.keywords if kw.arg == "expected_hash") == module.BPE_SHA, "BPE artifact pin differs")
        tokenizer.update(package_version=distribution.version, definition_path=str(path.resolve()), definition_sha256=hashlib.sha256(raw).hexdigest(), inspected_via="AST only; no BPE loader execution")

    check("first_record_ignores_poisoned_later_record", first_only)
    check("UTF8_crosses_one_MiB_boundary_without_loss", utf8_boundary)
    check("duplicate_JSON_keys_rejected", lambda: rejected(lambda: module.first_record(b'[{"x":1,"x":2},{}]')))
    check("nonfinite_JSON_rejected", lambda: rejected(lambda: module.first_record(b'[{"x":NaN},{}]')))
    check("missing_following_delimiter_rejected", lambda: rejected(lambda: module.first_record(b'[{"x":1}')))
    check("physical_alias_date_role_text_and_metadata_conservation", conservation)
    check("boolean_range_endpoint_rejected", range_type)
    check("tokenizer_definition_matches_installed_0_14_0", installed_definition)
    return {"schema_version": 1, "decision": "accept-c3-prefix-integrity-checks" if all(row["passed"] for row in checks) else "reject-c3-prefix-integrity-checks",
            "counter_path": str(counter.resolve()), "counter_sha256": hashlib.sha256(counter.read_bytes()).hexdigest(),
            "checker_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), "checks": checks,
            "check_count": len(checks), "failed_count": sum(not row["passed"] for row in checks), "tokenizer": tokenizer,
            "scope": "synthetic integrity checks only; no M data, counting experiment, semantic acceptance or score", "model_calls": 0, "paid_calls": 0}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--counter", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = run(args.counter)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x", encoding="utf-8") as handle:
        json.dump(result, handle, indent=2, allow_nan=False)
        handle.write("\n")
    print(json.dumps({"decision": result["decision"], "failed_count": result["failed_count"]}))
    return int(result["failed_count"] != 0)


if __name__ == "__main__":
    sys.exit(main())
