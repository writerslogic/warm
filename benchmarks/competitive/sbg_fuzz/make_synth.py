# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Synthetic benchmark-format corpus whose terms have exact document frequencies at limit-1, limit and limit+1
of each engine threshold, for the cases a real corpus does not contain.

  make_synth.py --docs N --seed S --threshold NAME=VALUE ... > corpus.jsonl

Each document is background words (zipf-like draw from 3,000 words, 30 to 120 per document) plus the special terms
assigned to it. A threshold VALUE is scaled to the corpus by the caller: pass SLOT_SHARE as ceil(docs / 32).
Output lines are `{"id": ..., "text": ...}` as build_index reads them.
"""

import argparse
import json
import random
import string


def letters(number):
    out = ""
    number += 1
    while number:
        number, rem = divmod(number - 1, 26)
        out = string.ascii_lowercase[rem] + out
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--docs", type=int, required=True)
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--threshold", action="append", default=[], help="NAME=VALUE")
    args = parser.parse_args()
    rng = random.Random(args.seed)
    background = ["w" + letters(i) for i in range(3000)]
    weights = [1.0 / (rank + 1) for rank in range(len(background))]
    special = [[] for _ in range(args.docs)]
    for spec in args.threshold:
        name, value = spec.split("=")
        for delta in (-1, 0, 1):
            df = int(value) + delta
            if 0 < df <= args.docs:
                term = "zz%s%s" % (letters(len(name) * 7 + sum(map(ord, name))), "m" if delta < 0 else "p" if delta > 0 else "e")
                term += letters(int(value))
                for doc in rng.sample(range(args.docs), df):
                    special[doc].append(term)
    for doc in range(args.docs):
        words = rng.choices(background, weights, k=rng.randint(30, 120)) + special[doc]
        rng.shuffle(words)
        print(json.dumps({"id": str(doc), "text": " ".join(words)}))


if __name__ == "__main__":
    main()
