# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Scalar E48 specification; no model, tokenizer, network, or world oracle."""

from dataclasses import dataclass
from datetime import date
import hashlib
import json
import math

INPUT_CAP = 4096
OUTPUT_CAP = 64
TEXT_BYTE_CAP = 1048576
ARMS = ("baseline", "global_only", "per_turn")
INSTRUCTION = (
    "Read the dated dialogue below. Answer the question using this dialogue. "
    "Return only the requested value, or unknown if it is not stated.\n\n"
)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def integer(value, lower, upper, message):
    require(type(value) is int and lower <= value <= upper, message)


@dataclass(frozen=True)
class Prompt:
    text: str
    sources: tuple
    spans: tuple
    question_start: int


@dataclass(frozen=True)
class Plan:
    token_ids: tuple
    owners: tuple
    question_start: int
    lengths: tuple
    source_digest: str

    @property
    def digest(self):
        fields = [self.source_digest, self.token_ids, self.owners, self.question_start, self.lengths]
        return hashlib.sha256(json.dumps(fields, separators=(",", ":")).encode()).hexdigest()


@dataclass(frozen=True)
class Cache:
    plan_digest: str
    token_ids: tuple

    @property
    def cursor(self):
        return len(self.token_ids)


def serialize(sources, question):
    require(type(sources) in (list, tuple) and 1 <= len(sources) <= INPUT_CAP, "source count")
    require(type(question) is str and question.strip(), "question")
    require(len(question) <= TEXT_BYTE_CAP, "question length")
    text, originals, spans = INSTRUCTION, [], []
    for source in sources:
        require(type(source) is dict and set(source) == {"role", "date", "content"}, "source fields")
        role, observed, content = source["role"], source["date"], source["content"]
        require(role in ("user", "assistant") and type(role) is str, "source role")
        require(type(observed) is str and date.fromisoformat(observed).isoformat() == observed, "source date")
        require(type(content) is str and content.strip(), "source content")
        require(len(content) <= TEXT_BYTE_CAP, "source content length")
        block = f"[{observed}] {role}\n{content}\n\n"
        spans.append((len(text), len(text) + len(block)))
        text += block
        originals.append((role, observed, content))
        require(len(text.encode("utf-8")) <= TEXT_BYTE_CAP, "source byte cap")
    question_start = len(text)
    text += f"Question: {question}\nAnswer:"
    require(len(text.encode("utf-8")) <= TEXT_BYTE_CAP, "prompt byte cap")
    return Prompt(text, tuple(originals), tuple(spans), question_start)


def group_bias(lengths, arm):
    require(arm in ARMS, "attention arm")
    require(type(lengths) is tuple and 1 <= len(lengths) <= INPUT_CAP, "group count")
    for length in lengths:
        integer(length, 1, INPUT_CAP, "group length")
    require(sum(lengths) <= INPUT_CAP, "source token cap")
    if arm == "baseline":
        return (0.0,) * len(lengths)
    mean_log = math.log(sum(lengths) / len(lengths))
    biases = tuple(mean_log - math.log(length) for length in lengths)
    if arm == "global_only":
        return (math.fsum(biases) / len(biases),) * len(biases)
    return biases


def token_plan(prompt, token_ids, offsets):
    """Offsets cover exact serialized characters; ambiguous boundary tokens reject."""
    require(type(prompt) is Prompt, "prompt type")
    require(type(token_ids) in (list, tuple) and 1 <= len(token_ids) <= INPUT_CAP, "input token cap")
    require(type(offsets) in (list, tuple) and len(offsets) == len(token_ids), "offset count")
    for token in token_ids:
        integer(token, 0, 2**31 - 1, "token id")
    owners, lengths, cursor, group, question_start = [], [0] * len(prompt.spans), 0, 0, None
    segments = [(0, prompt.spans[0][0], None)]
    segments.extend((start, end, index) for index, (start, end) in enumerate(prompt.spans))
    segments.append((prompt.question_start, len(prompt.text), None))
    for position, pair in enumerate(offsets):
        require(type(pair) in (list, tuple) and len(pair) == 2, "token offset")
        start, end = pair
        integer(start, 0, len(prompt.text) - 1, "token offset start")
        integer(end, start + 1, len(prompt.text), "token offset end")
        require(start == cursor, "token offsets must cover source exactly once")
        while group < len(segments) and start >= segments[group][1]:
            group += 1
        require(group < len(segments) and end <= segments[group][1], "token crosses source boundary")
        owner = segments[group][2]
        owners.append(owner)
        if owner is not None:
            lengths[owner] += 1
        if start == prompt.question_start:
            question_start = position
        cursor = end
    require(cursor == len(prompt.text) and question_start is not None, "incomplete prompt ownership")
    group_bias(tuple(lengths), "baseline")
    payload = [prompt.text, prompt.sources, prompt.spans, list(token_ids), owners, question_start]
    digest = hashlib.sha256(json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8")).hexdigest()
    return Plan(tuple(token_ids), tuple(owners), question_start, tuple(lengths), digest)


def new_cache(plan):
    return Cache(plan.digest, ())


def validate_cache(plan, cache):
    require(type(plan) is Plan and type(cache) is Cache, "state type")
    require(len(plan.token_ids) == len(plan.owners) and 1 <= len(plan.token_ids) <= INPUT_CAP, "plan shape")
    integer(plan.question_start, 1, len(plan.token_ids) - 1, "plan question boundary")
    group_bias(plan.lengths, "baseline")
    counts = [0] * len(plan.lengths)
    for position, owner in enumerate(plan.owners):
        if owner is not None:
            integer(owner, 0, len(counts) - 1, "source owner")
            require(position < plan.question_start, "source owner in question")
            counts[owner] += 1
    require(tuple(counts) == plan.lengths, "ownership lengths changed")
    require(cache.plan_digest == plan.digest, "stale source ownership")
    require(type(cache.token_ids) is tuple and cache.cursor <= len(plan.token_ids) + OUTPUT_CAP, "cache capacity")
    for token in cache.token_ids:
        integer(token, 0, 2**31 - 1, "cached token id")
    prefix = min(cache.cursor, len(plan.token_ids))
    require(cache.token_ids[:prefix] == plan.token_ids[:prefix], "cache prefix changed")


def append(plan, cache, token_ids, *, offset, phase):
    validate_cache(plan, cache)
    integer(offset, 0, INPUT_CAP + OUTPUT_CAP, "cache offset")
    require(offset == cache.cursor, "stale cache cursor")
    require(type(token_ids) in (list, tuple) and token_ids, "empty append")
    for token in token_ids:
        integer(token, 0, 2**31 - 1, "appended token id")
    end = offset + len(token_ids)
    require(phase in ("history", "query", "decode"), "append phase")
    if phase == "history":
        require(end <= plan.question_start, "history crossed question boundary")
    elif phase == "query":
        require(plan.question_start <= offset < end <= len(plan.token_ids), "query offset")
    else:
        require(len(plan.token_ids) <= offset < end <= len(plan.token_ids) + OUTPUT_CAP, "decode offset")
    if phase != "decode":
        require(tuple(token_ids) == plan.token_ids[offset:end], "prefill tokens changed")
    return Cache(plan.digest, cache.token_ids + tuple(token_ids))


def attention_row(plan, cache, query_position, arm):
    validate_cache(plan, cache)
    integer(query_position, 0, cache.cursor - 1, "query position")
    biases = group_bias(plan.lengths, arm)
    active = query_position >= plan.question_start
    row = []
    for key in range(cache.cursor):
        if key > query_position:
            row.append(-math.inf)
        elif active and key < len(plan.owners) and plan.owners[key] is not None:
            row.append(biases[plan.owners[key]])
        else:
            row.append(0.0)
    return tuple(row)


def probabilities(logits, additive_row):
    require(len(logits) == len(additive_row) and 1 <= len(logits) <= INPUT_CAP + OUTPUT_CAP, "attention shape")
    require(all(type(value) in (int, float) and math.isfinite(value) for value in logits), "attention logits")
    require(all(type(value) in (int, float) and (math.isfinite(value) or value == -math.inf)
                for value in additive_row), "attention mask")
    adjusted = [score + bias for score, bias in zip(logits, additive_row)]
    maximum = max(adjusted)
    require(math.isfinite(maximum), "all keys excluded")
    masses = [math.exp(value - maximum) for value in adjusted]
    total = math.fsum(masses)
    return tuple(value / total for value in masses)


def world_sign_flip(differences):
    """Two-sided exact test under independent, sign-symmetric world differences."""
    require(type(differences) in (list, tuple) and len(differences) == 32, "world count")
    for difference in differences:
        integer(difference, -4, 4, "world error difference")
    distribution = {0: 1}
    for difference in differences:
        updated = {}
        for total, count in distribution.items():
            for signed in (difference, -difference):
                updated[total + signed] = updated.get(total + signed, 0) + count
        distribution = updated
    observed = sum(differences)
    tail = sum(count for total, count in distribution.items() if abs(total) >= abs(observed))
    denominator = 2**32
    require(sum(distribution.values()) == denominator, "sign-flip mass")
    return {"worlds": 32, "observed_baseline_minus_candidate_errors": observed,
            "two_sided_tail_count": tail, "assignment_count": denominator,
            "p_value": tail / denominator, "p_le_0_05": 20 * tail <= denominator}
