# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Independent generated-world oracle and ordinary-dialogue renderer for H-C-5."""

from dataclasses import dataclass
from datetime import date, timedelta
import hashlib
import random
import re

VARIANTS = ("short_competitor", "verbose_competitor", "visible_neutral", "long_decisive")
FAMILIES = ("speaker_attribution", "later_update", "relation_date_scope", "two_source_combination")
REPETITIONS = 24
MAX_NEUTRAL_WORDS = 2048
NAMES = ("Adira", "Belen", "Cato", "Dara", "Elian", "Fara", "Galen", "Hana",
         "Iona", "Jalen", "Kira", "Luan", "Mira", "Neri", "Oren", "Pia",
         "Ravi", "Sela", "Tavi", "Una", "Vera", "Wren", "Yara", "Zeno")
VALUES = ("amber", "blue", "coral", "green", "indigo", "ivory", "jade", "lilac",
          "ochre", "orange", "pink", "plum", "red", "silver", "teal", "violet")


def require(condition, message):
    if not condition:
        raise ValueError(message)


@dataclass(frozen=True)
class Event:
    subject: str
    field: str
    value: str
    observed: str
    scope_date: str | None = None


@dataclass(frozen=True)
class Request:
    subject: str
    fields: tuple
    as_of: str
    scope_date: str | None = None


@dataclass(frozen=True)
class World:
    ordinal: int
    family: str
    events: tuple
    request: Request
    competitor: int
    decisive: int


def worlds(seed):
    require(type(seed) is str and re.fullmatch(r"[0-9a-f]{64}", seed) is not None, "seed must be 32 bytes of lowercase hex")
    result = []
    for ordinal in range(32):
        stream = hashlib.sha256(bytes.fromhex(seed) + ordinal.to_bytes(4, "big")).digest()
        rng = random.Random(int.from_bytes(stream, "big"))
        subject, other = rng.sample(NAMES, 2)
        a, b, c, d = rng.sample(VALUES, 4)
        start = date(2031, 1, 1) + timedelta(days=rng.randrange(365))
        days = tuple((start + timedelta(days=index)).isoformat() for index in range(4))
        family = FAMILIES[ordinal % 4]
        if family == "speaker_attribution":
            events = (Event(subject, "badge color", a, days[0]), Event(other, "badge color", b, days[1]))
            request, competitor, decisive = Request(subject, ("badge color",), days[3]), 1, 0
        elif family == "later_update":
            events = (Event(subject, "badge color", a, days[0]), Event(subject, "badge color", b, days[1]),
                      Event(other, "badge color", c, days[2]))
            request, competitor, decisive = Request(subject, ("badge color",), days[3]), 0, 1
        elif family == "relation_date_scope":
            events = (Event(subject, "badge color", a, days[2], days[0]),
                      Event(subject, "badge color", b, days[2], days[1]),
                      Event(subject, "locker color", c, days[2], days[0]))
            request, competitor, decisive = Request(subject, ("badge color",), days[3], days[0]), 1, 0
        else:
            events = (Event(subject, "badge color", a, days[0]), Event(subject, "locker color", b, days[1]),
                      Event(other, "badge color", c, days[2]), Event(other, "locker color", d, days[2]))
            request, competitor, decisive = Request(subject, ("badge color", "locker color"), days[3]), 2, 1
        result.append(World(ordinal, family, events, request, competitor, decisive))
    return tuple(result)


def truth(world):
    """Evaluate structured facts, never the renderer's text, repetitions, or token groups."""
    answers, evidence = [], []
    for field in world.request.fields:
        matching = [(index, event) for index, event in enumerate(world.events)
                    if event.subject == world.request.subject and event.field == field
                    and event.scope_date == world.request.scope_date and event.observed <= world.request.as_of]
        if not matching:
            answers.append("unknown")
            continue
        latest = max(event.observed for _, event in matching)
        chosen = [(index, event) for index, event in matching if event.observed == latest]
        require(len({event.value for _, event in chosen}) == 1, "ambiguous generated truth")
        answers.append(chosen[0][1].value)
        evidence.extend(index for index, _ in chosen)
    return {"answer": " | ".join(answers), "event_indices": evidence}


def assertion(event):
    if event.scope_date is None:
        return f"I am {event.subject}. My {event.field} is {event.value}."
    return f"I am {event.subject}. On {event.scope_date}, my {event.field} was {event.value}."


def question(request):
    when = f"on {request.scope_date}" if request.scope_date else f"as of {request.as_of}"
    if len(request.fields) == 1:
        return f"What was {request.subject}'s {request.fields[0]} {when}?"
    fields = " and ".join(request.fields)
    return f"What were {request.subject}'s {fields} {when}? Return those values in that order, separated by |."


def render(world, variant, *, neutral_words=0):
    """Public payload only; the oracle is not called and proof fields are not accepted."""
    require(variant in VARIANTS, "variant")
    require(type(neutral_words) is int and 0 <= neutral_words <= MAX_NEUTRAL_WORDS, "neutral length")
    sources = []
    for index, event in enumerate(world.events):
        content = assertion(event)
        if ((variant == "verbose_competitor" and index == world.competitor)
                or (variant == "long_decisive" and index == world.decisive)):
            content += (" To repeat the same statement: " + assertion(event)) * REPETITIONS
        if variant == "visible_neutral" and index == world.competitor:
            content += " The following words are neutral filler:" + " filler" * neutral_words + "."
        sources.append({"role": "user", "date": event.observed, "content": content})
    return {"sources": sources, "question": question(world.request)}


def matched_renderings(world, measure):
    """First exact native-token match only; inability to match rejects this grammar."""
    def validated_measure(payload):
        value = measure(payload)
        require(type(value) is tuple and len(value) == 2, "measurement shape")
        total, lengths = value
        require(type(total) is int and 1 <= total <= 4096 and type(lengths) is tuple
                and len(lengths) == len(world.events)
                and all(type(length) is int and length > 0 for length in lengths)
                and sum(lengths) <= total, "invalid source-token measurement")
        return value

    result = {variant: render(world, variant) for variant in VARIANTS if variant != "visible_neutral"}
    target = validated_measure(result["verbose_competitor"])
    for neutral_words in range(MAX_NEUTRAL_WORDS + 1):
        candidate = render(world, "visible_neutral", neutral_words=neutral_words)
        if validated_measure(candidate) == target:
            result["visible_neutral"] = candidate
            return tuple((variant, result[variant]) for variant in VARIANTS)
    raise ValueError("no equal-token visible-neutral control within fixed grammar")


def dataset(seed, measure):
    """Returns disjoint reader/oracle records; no answers are embedded in reader records."""
    reader_records, oracle_records = [], []
    for world in worlds(seed):
        answer = truth(world)
        for variant, payload in matched_renderings(world, measure):
            identity = {"world": world.ordinal, "variant": variant}
            reader_records.append({**identity, "payload": payload})
            oracle_records.append({**identity, **answer})
    return reader_records, oracle_records
