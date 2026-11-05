#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Unary change-report cue tagger (English only), frozen before labeling.

A statement is flagged when it contains a lexical cue that it reports a change to a prior state
of the speaker (switched, moved, no longer, stopped, quit, started, used to, changed, now, again)
or an anchor-relative onset (since/ever since/after + event, "these days", "lately").
Protocol: docs/research/cue-tags.md.
"""
import re

_F = re.IGNORECASE

# Group "lex": change verbs and phrases.
LEX = [
    r"\bswitch(?:ed|ing)?\s+(?:to|over|from|back|jobs|careers|teams)\b",
    r"\bmov(?:ed|ing)\s+(?:to|into|out|over|back|away|in\b|from|across)",
    r"\bjust\s+moved\b",
    r"\brelocat(?:ed|ing)\b",
    r"\b(?:have|has|'ve|'s|i|we)\s+changed\b",
    r"\bchanged\s+(?:my|our|jobs|careers|teams|to)\b",
    r"\bno\s+longer\b",
    r"\bnot\s+any\s?more\b",
    r"\bany\s?more\b",
    r"\bused\s+to\b",
    r"\b(?:quit|quitting)\b",
    r"\bstopped\b",
    r"\bgave\s+up\b",
    r"\bgiving\s+up\b",
    r"\b(?:i|we|i've|we've|i've just|recently|just|finally)\s+(?:started|began|begun)\b",
    r"\bstarted\s+(?:a\s+new|my\s+new|working|using|going|taking|to|on\s+a)\b",
    r"\btook\s+up\b",
    r"\bpicked\s+up\s+(?:a|the)\s+(?:new\s+)?(?:habit|hobby)\b",
    r"\btransition(?:ed|ing)\s+(?:to|from|into|out)\b",
    r"\b(?:replaced|swapped|traded\s+in|got\s+rid\s+of|ditched|sold\s+my)\b",
    r"\b(?:upgraded|downgraded)\b",
    r"\bcut\s+(?:back|out|down)\b",
    r"\binstead\s+of\b",
    r"\bnow\s+that\b",
    r"\b(?:am|'m|is|'s|are|'re|have|'ve|has|was|were)\s+now\b",
    r"(?<!^)(?<![.!?]\s)(?<!right\s)(?<!for\s)(?<!by\s)(?<!until\s)(?<!from\s)(?<!up\sto\s)\bnow\b(?!\s*,)(?!\s+and\s+then)",
    r"\b(?:back\s+to|again)\b",
    r"\bfinally\s+(?:got|finished|moved|switched|bought|made|started|have)\b",
    r"\b(?:previously|formerly)\b",
    r"\bnew\s+(?:job|role|position|apartment|house|place|city|car|phone|routine|schedule|diet)\b",
]

# Group "tense": anchor-relative onset of a current state.
TENSE = [
    r"\b(?:ever\s+)?since\s+(?:i|we|my|our|the)\b",
    r"\bafter\s+(?:i|we)\s+(?:moved|switched|started|quit|stopped|changed|got|lost|left)\b",
    r"\b(?:these\s+days|nowadays|lately)\b",
    r"\b(?:i|we)(?:'ve|\s+have)\s+(?:recently|just)\b",
]

_LEX = [re.compile(p, _F | re.MULTILINE) for p in LEX]
_TENSE = [re.compile(p, _F | re.MULTILINE) for p in TENSE]


def _norm(text: str) -> str:
    return text.replace("’", "'").replace("‘", "'")


def hits(text: str) -> dict:
    """Which patterns fire, by group (index into LEX / TENSE)."""
    t = _norm(text)
    return {
        "lex": [i for i, r in enumerate(_LEX) if r.search(t)],
        "tense": [i for i, r in enumerate(_TENSE) if r.search(t)],
    }


def flagged(text: str) -> bool:
    h = hits(text)
    return bool(h["lex"] or h["tense"])
