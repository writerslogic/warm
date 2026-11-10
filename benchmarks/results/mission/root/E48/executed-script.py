# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Independent scalar/source integrity checks for E48; never invokes inference."""

import argparse
from dataclasses import replace
import hashlib
import importlib.util
import itertools
import json
import math
import os
from pathlib import Path
import re
import sys
import tempfile


def require(condition, message):
    if not condition:
        raise ValueError(message)


def close(left, right):
    require(math.isclose(left, right, rel_tol=2e-13, abs_tol=2e-14), "scalar oracle disagreement")


def rejected(call):
    try:
        call()
    except ValueError:
        return
    raise ValueError("invalid boundary accepted")


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def run(attention_path, generator_path):
    attention = load(attention_path, "e48_attention_under_check")
    generator = load(generator_path, "e48_worlds_under_check")
    checks = []

    def check(name, action):
        try:
            action()
            checks.append({"name": name, "passed": True})
        except Exception as error:
            checks.append({"name": name, "passed": False, "error_type": type(error).__name__, "error": str(error)})

    def character_plan(payload):
        prompt = attention.serialize(payload["sources"], payload["question"])
        plan = attention.token_plan(prompt, [ord(char) for char in prompt.text],
                                    [(index, index + 1) for index in range(len(prompt.text))])
        return prompt, plan

    payload = {"sources": [{"role": "user", "date": "2034-03-01", "content": "I kept λ and 🙂."},
                            {"role": "assistant", "date": "2034-03-02", "content": "I kept λ and 🙂."}],
               "question": "Which dated message came later?"}

    def group_boundaries():
        for lengths in ((1,), (4096,), (1, 4094), (1, 4095), (1, 1), (17, 17, 17)):
            biases = attention.group_bias(lengths, "per_turn")
            expected_mass = sum(lengths) / len(lengths)
            for length, bias in zip(lengths, biases):
                close(length * math.exp(bias), expected_mass)
            close(sum(length * math.exp(bias) for length, bias in zip(lengths, biases)), sum(lengths))
            if len(set(lengths)) == 1:
                require(biases == (0.0,) * len(lengths), "equal-length bias is not zero")
                require(attention.group_bias(lengths, "global_only") == biases, "equal-length control differs")
        for lengths in ((), (0,), (-1,), (True,), (1.0,), (4097,), (1, 4096)):
            rejected(lambda lengths=lengths: attention.group_bias(lengths, "per_turn"))

    def independent_softmax():
        lengths = (1, 4, 9)
        owners = (0,) + (1,) * 4 + (2,) * 9 + (None, None)
        scores = tuple(((index * 7) % 13 - 6) / 3 for index in range(len(owners)))
        biases = attention.group_bias(lengths, "per_turn")
        row = tuple(0.0 if owner is None else biases[owner] for owner in owners)
        actual = attention.probabilities(scores, row)
        weights = [math.exp(score) * (1 if owner is None else sum(lengths) / (len(lengths) * lengths[owner]))
                   for score, owner in zip(scores, owners)]
        for left, right in zip(actual, [weight / math.fsum(weights) for weight in weights]):
            close(left, right)
        tied = attention.probabilities((0.0,) * len(owners), row)
        for group in range(3):
            close(math.fsum(value for value, owner in zip(tied, owners) if owner == group), (14 / 3) / 16)
        close(tied[-1], 1 / 16)
        global_bias = attention.group_bias(lengths, "global_only")
        require(len(set(global_bias)) == 1, "global control became per-turn")
        close(math.exp(global_bias[0]), ((14 / 3) ** 3 / (1 * 4 * 9)) ** (1 / 3))
        rejected(lambda: attention.probabilities((0.0,), (-math.inf,)))
        rejected(lambda: attention.probabilities((math.nan,), (0.0,)))

    def source_ownership():
        prompt, plan = character_plan(payload)
        require(prompt.sources == tuple((source["role"], source["date"], source["content"])
                                        for source in payload["sources"]), "role/date/text changed")
        require(len(plan.lengths) == 2 and set(plan.owners) == {None, 0, 1}, "physical occurrences merged")
        for group, (start, end) in enumerate(prompt.spans):
            source = payload["sources"][group]
            require(prompt.text[start:end] == f'[{source["date"]}] {source["role"]}\n{source["content"]}\n\n', "source serialization changed")
            require(plan.owners[start:end] == (group,) * (end - start), "token ownership misplaced")
        require(plan.owners[plan.question_start:] == (None,) * (len(plan.token_ids) - plan.question_start), "question became source")
        offsets = [(index, index + 1) for index in range(len(prompt.text))]
        start = prompt.spans[0][0]
        crossed = offsets[:start - 1] + [(start - 1, start + 1)] + offsets[start + 1:]
        rejected(lambda: attention.token_plan(prompt, [1] * len(crossed), crossed))
        missing = offsets[:start] + offsets[start + 1:]
        rejected(lambda: attention.token_plan(prompt, [1] * len(missing), missing))
        poisoned = [{**payload["sources"][0], "has_answer": True}]
        rejected(lambda: attention.serialize(poisoned, payload["question"]))
        rejected(lambda: attention.token_plan(prompt, [1], [(0, 0)]))
        rejected(lambda: attention.token_plan(prompt, [True] * len(offsets), offsets))

    def split_and_causal():
        _, plan = character_plan(payload)
        cache = attention.new_cache(plan)
        one = attention.append(plan, cache, plan.token_ids[:plan.question_start], offset=0, phase="history")
        one = attention.append(plan, one, plan.token_ids[plan.question_start:], offset=one.cursor, phase="query")
        split = cache
        for end in (1, 19, plan.question_start):
            split = attention.append(plan, split, plan.token_ids[split.cursor:end], offset=split.cursor, phase="history")
        for end in (plan.question_start + 1, len(plan.token_ids)):
            split = attention.append(plan, split, plan.token_ids[split.cursor:end], offset=split.cursor, phase="query")
        require(one == split and split.cursor == len(plan.token_ids), "split prefill cursor changed")
        split = attention.append(plan, split, [101, 102], offset=split.cursor, phase="decode")
        one = attention.append(plan, one, [101], offset=one.cursor, phase="decode")
        one = attention.append(plan, one, [102], offset=one.cursor, phase="decode")
        require(one == split, "split decode cursor changed")
        for arm in attention.ARMS:
            for query in (0, plan.question_start - 1, plan.question_start, len(plan.token_ids), one.cursor - 1):
                row = attention.attention_row(plan, one, query, arm)
                require(row == attention.attention_row(plan, split, query, arm), "split attention differs")
                expected = []
                mean = sum(plan.lengths) / len(plan.lengths)
                uniform_factor = math.prod(mean / length for length in plan.lengths) ** (1 / len(plan.lengths))
                for key in range(one.cursor):
                    if key > query:
                        require(row[key] == -math.inf, "future key leaked")
                        expected.append(0.0)
                        continue
                    owner = plan.owners[key] if key < len(plan.owners) else None
                    factor = 1.0
                    if query >= plan.question_start and owner is not None:
                        if arm == "per_turn":
                            factor = mean / plan.lengths[owner]
                        elif arm == "global_only":
                            factor = uniform_factor
                    expected.append(factor)
                logits = tuple(1e100 if key > query else 0.0 for key in range(one.cursor))
                probabilities = attention.probabilities(logits, row)
                for actual, mass in zip(probabilities, expected):
                    close(actual, mass / math.fsum(expected))
                require(all(probabilities[key] == 0.0 for key in range(query + 1, one.cursor)), "future probability nonzero")

    def adversarial_state():
        _, plan = character_plan(payload)
        empty = attention.new_cache(plan)
        rejected(lambda: attention.append(plan, empty, [1], offset=True, phase="history"))
        rejected(lambda: attention.append(plan, empty, [1], offset=1, phase="history"))
        rejected(lambda: attention.append(plan, empty, [1], offset=0, phase="decode"))
        rejected(lambda: attention.append(plan, empty, plan.token_ids, offset=0, phase="history"))
        history = attention.append(plan, empty, plan.token_ids[:plan.question_start], offset=0, phase="history")
        changed = {"sources": [{**payload["sources"][0], "date": "2034-03-03"}, payload["sources"][1]],
                   "question": payload["question"]}
        _, other = character_plan(changed)
        rejected(lambda: attention.attention_row(other, history, 0, "baseline"))
        swapped = tuple(1 - owner if owner is not None else None for owner in plan.owners)
        mutated = replace(plan, owners=swapped, lengths=tuple(reversed(plan.lengths)))
        rejected(lambda: attention.attention_row(mutated, history, 0, "baseline"))
        wrong_tokens = attention.Cache(plan.digest, (0,) + history.token_ids[1:])
        rejected(lambda: attention.attention_row(plan, wrong_tokens, 0, "baseline"))
        full = attention.append(plan, history, plan.token_ids[plan.question_start:], offset=history.cursor, phase="query")
        limit_minus_one = attention.append(plan, full, [1] * 63, offset=full.cursor, phase="decode")
        limit = attention.append(plan, limit_minus_one, [1], offset=limit_minus_one.cursor, phase="decode")
        require(limit.cursor == len(plan.token_ids) + 64, "decode capacity shrank")
        rejected(lambda: attention.append(plan, limit, [1], offset=limit.cursor, phase="decode"))

    def clustered_statistic():
        cases = ((4, -3, 2, -1), (4, 4, -4, 0), (1, 1, 1, -1), (0, 0, 0, 0))
        for first_four in cases:
            observed = abs(sum(first_four))
            tail = sum(abs(sum(sign * value for sign, value in zip(signs, first_four))) >= observed
                       for signs in itertools.product((-1, 1), repeat=4))
            result = attention.world_sign_flip(first_four + (0,) * 28)
            require(result["two_sided_tail_count"] == tail * 2**28 and result["assignment_count"] == 2**32,
                    "cluster DP differs from independent enumeration")
        require(attention.world_sign_flip((4,) + (0,) * 31)["p_value"] == 1.0, "four correlated variants treated as independent")
        require(attention.world_sign_flip((0,) * 32)["p_value"] == 1.0, "zero differences rejected")
        require(attention.world_sign_flip((1,) * 32)["two_sided_tail_count"] == 2, "extreme sign-flip tail")
        require(not attention.world_sign_flip((1,) * 5 + (0,) * 27)["p_le_0_05"], "threshold above .05 accepted")
        require(attention.world_sign_flip((1,) * 6 + (0,) * 26)["p_le_0_05"], "threshold below .05 rejected")
        for differences in ((0,) * 31, (0,) * 33, (True,) + (0,) * 31, (5,) + (0,) * 31):
            rejected(lambda differences=differences: attention.world_sign_flip(differences))

    def independent_world_truth():
        event = generator.Event
        request = generator.Request("Ada", ("badge color", "locker color"), "2033-01-05")
        world = generator.World(0, "manual", (event("Ada", "badge color", "red", "2033-01-01"),
            event("Ada", "badge color", "blue", "2033-01-02"), event("Bex", "badge color", "green", "2033-01-03"),
            event("Ada", "locker color", "silver", "2033-01-03"), event("Ada", "badge color", "pink", "2033-01-06")),
            request, 0, 1)
        require(generator.truth(world) == {"answer": "blue | silver", "event_indices": [1, 3]}, "independent world truth disagrees")
        missing = replace(world, request=replace(request, subject="Unmentioned"))
        require(generator.truth(missing)["answer"] == "unknown | unknown", "closed-world unknown changed")
        scoped = replace(world, request=replace(request, fields=("badge color",), scope_date="2033-01-01"))
        require(generator.truth(scoped)["answer"] == "unknown", "scope erased")
        ambiguous = replace(world, events=world.events + (event("Ada", "badge color", "gold", "2033-01-02"),))
        rejected(lambda: generator.truth(ambiguous))
        original = generator.truth
        generator.truth = lambda _: (_ for _ in ()).throw(ValueError("oracle called by renderer"))
        try:
            generator.render(world, "short_competitor")
        finally:
            generator.truth = original

    def generated_source_conservation():
        worlds = generator.worlds("a6" * 32)
        require(len(worlds) == 32 and len({world.ordinal for world in worlds}) == 32, "world coverage")
        require(all(sum(world.family == family for world in worlds) == 8 for family in generator.FAMILIES), "family coverage")
        pattern = re.compile(r"I am ([A-Za-z]+)\. (?:My (badge color|locker color) is ([a-z]+)|On (\d{4}-\d{2}-\d{2}), my (badge color|locker color) was ([a-z]+))\.")

        def word_measure(public):
            lengths = tuple(2 + len(source["content"].split()) for source in public["sources"])
            return (10 + len(public["question"].split()) + sum(lengths), lengths)

        for world in worlds:
            rendered = dict(generator.matched_renderings(world, word_measure))
            require(word_measure(rendered["verbose_competitor"]) == word_measure(rendered["visible_neutral"]), "control length matcher changed")
            expected = generator.truth(world)["answer"]
            for variant, public in rendered.items():
                require(set(public) == {"sources", "question"}, "truth metadata entered reader input")
                single = re.fullmatch(r"What was ([A-Za-z]+)'s (badge color|locker color) (as of|on) (\d{4}-\d{2}-\d{2})\?", public["question"])
                multiple = re.fullmatch(r"What were ([A-Za-z]+)'s (badge color) and (locker color) as of (\d{4}-\d{2}-\d{2})\? Return those values in that order, separated by \|\.", public["question"])
                require(single is not None or multiple is not None, "question grammar changed")
                if single:
                    subject, field, mode, when = single.groups()
                    require(subject == world.request.subject and (field,) == world.request.fields
                            and mode == ("on" if world.request.scope_date else "as of")
                            and when == (world.request.scope_date or world.request.as_of), "rendered request differs from world")
                else:
                    subject, first, second, when = multiple.groups()
                    require(subject == world.request.subject and (first, second) == world.request.fields
                            and when == world.request.as_of and world.request.scope_date is None, "rendered combination request differs")
                require(len(public["sources"]) == len(world.events), "physical event omitted")
                interpreted = {}
                for index, (source, event) in enumerate(zip(public["sources"], world.events)):
                    require(set(source) == {"role", "date", "content"}, "oracle field in source")
                    require(source["role"] == "user" and source["date"] == event.observed, "role/date changed")
                    matches = list(pattern.finditer(source["content"]))
                    require(matches and matches[0].start() == 0, "original assertion lost")
                    repeated = ((variant == "verbose_competitor" and index == world.competitor)
                                or (variant == "long_decisive" and index == world.decisive))
                    require(len(matches) == (25 if repeated else 1), "unexpected fact added or removed")
                    for match in matches:
                        subject, field, value, scope, scoped_field, scoped_value = match.groups()
                        fact = (subject, field or scoped_field, scope)
                        value = value or scoped_value
                        prior = interpreted.get(fact)
                        if source["date"] <= world.request.as_of and (prior is None or source["date"] >= prior[0]):
                            interpreted[fact] = (source["date"], value)
                actual = " | ".join(interpreted.get((world.request.subject, field, world.request.scope_date), (None, "unknown"))[1]
                                    for field in world.request.fields)
                require(actual == expected, "independent rendered-source interpretation disagrees with world truth")
            long_payload = rendered["long_decisive"]
            prompt, plan = character_plan(long_payload)
            require(len(plan.lengths) == len(world.events) and sum(plan.lengths) == sum(end - start for start, end in prompt.spans),
                    "long decisive source lost during ownership")
        first = worlds[0]
        rejected(lambda: generator.matched_renderings(first, lambda public: (1, (1,))))
        for seed in ("", "a6" * 31, "A6" * 32, True):
            rejected(lambda seed=seed: generator.worlds(seed))

    for name, action in (("group_length_boundaries_and_tied_mass", group_boundaries),
                         ("independent_scalar_softmax_and_global_control", independent_softmax),
                         ("physical_source_unicode_and_boundary_ownership", source_ownership),
                         ("split_prefill_decode_offsets_and_future_exclusion", split_and_causal),
                         ("stale_ownership_cursor_and_output_boundaries", adversarial_state),
                         ("exact_world_cluster_sign_flip", clustered_statistic),
                         ("independent_world_truth_and_renderer_separation", independent_world_truth),
                         ("all_32_worlds_four_variants_and_long_decisive_conservation", generated_source_conservation)):
        check(name, action)
    failures = sum(not check["passed"] for check in checks)
    return {"experiment": "E48", "decision": "accept-scalar-source-only" if failures == 0 else "reject-scalar-source-gate",
            "checks": checks, "failures": failures, "benchmark_items": 0, "model_calls": 0,
            "attention_sha256": hashlib.sha256(attention_path.read_bytes()).hexdigest(),
            "generator_sha256": hashlib.sha256(generator_path.read_bytes()).hexdigest(),
            "checker_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            "limits": ["Scalar arithmetic and closed-grammar generated sources only; no reader-quality result.",
                       "Character offsets and whitespace measurement are independent test instruments, not native Qwen tokenization.",
                       "Native tokenizer/chat-template ownership and equal-token controls remain prerequisites before inference.",
                       "Sign-flip inference requires independent worlds and conditional sign symmetry/exchangeability under the null."]}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--attention", required=True, type=Path)
    parser.add_argument("--generator", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    require(not args.out.exists(), "output exists")
    result = run(args.attention, args.generator)
    descriptor, temporary = tempfile.mkstemp(prefix=".E48-", dir=args.out.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(result, handle, indent=2, ensure_ascii=False, allow_nan=False)
            handle.write("\n")
        os.link(temporary, args.out)
    finally:
        os.unlink(temporary)
    print(json.dumps({"decision": result["decision"], "failures": result["failures"], "out": str(args.out)}))
    return int(result["failures"] != 0)


if __name__ == "__main__":
    raise SystemExit(main())
