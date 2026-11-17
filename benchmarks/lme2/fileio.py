# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""File helpers that close what they open: the stage scripts read JSON, JSON lines and pickles and
append one JSON line per finished question."""
import json, pickle


def read_json(p):
    with open(p) as f:
        return json.load(f)


def write_json(obj, p):
    with open(p, "w") as f:
        json.dump(obj, f)


def read_pickle(p):
    with open(p, "rb") as f:
        return pickle.load(f)


def lines(p):
    """The lines of a text file, read lazily; the file closes when the iteration ends."""
    with open(p) as f:
        yield from f


def append_line(p, text):
    with open(p, "a") as f:
        f.write(text)
