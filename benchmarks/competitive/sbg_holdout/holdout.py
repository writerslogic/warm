# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Held-out query sets for the search-benchmark-game work. The rules live in MANIFEST.md; this file implements them.

  holdout.py candidates --raw NQ.jsonl --seed N [--sample N] --out CAND.jsonl --phrases PHRASES.txt
  holdout.py build-nq --candidates CAND.jsonl --counts COUNTS.txt --seed N --out SET.jsonl [--exclude FILE ...]
  holdout.py build-random --vocab VOCAB.tsv --corpus CORPUS.jsonl --seed N --sources N --out SET.jsonl [--exclude FILE ...]
  holdout.py build-sweep --vocab VOCAB.tsv --seed N --per-length N --out SET.jsonl
  holdout.py build-long --books FILE [FILE ...] --seed N --count N --out SET.jsonl [--exclude FILE ...]
  holdout.py summary FILE [FILE ...]

`COUNTS.txt` holds one phrase hit count per line of PHRASES.txt, in order, from the engine's COUNT over the
index (the `count_queries` binary of ../sbg_fuzz). Sets use the format of the benchmark's queries-full.txt.
"""

import argparse
import collections
import hashlib
import json
import re
import sys

# Union lengths of the query-length sweep (impact.rs MAX_TERMS is 64).
SWEEP_LENGTHS = (1, 2, 4, 8, 16, 32, 48, 63, 64, 65, 96, 128)
# The benchmark corpus transform (corpus_transform.py): lowercase, every run of non-letters becomes one space.
NON_LETTERS = re.compile("[^a-zA-Z]+")
SENTENCE_END = re.compile(r"(?<=[.!?])\s+")
LONG_MIN, LONG_MAX = 20, 60

# The benchmark's own rules (format_queries.py): the source query must be letters and spaces only, and is
# split on whitespace.
LETTERS_ONLY = re.compile("^[a-z ]+$")
WHITESPACE = re.compile(r"\s+")
CATEGORIES = ("term", "intersection", "phrase", "union", "intersection_union", "negated")
# df strata, inclusive. The last stratum is the 100 longest lists by rank, not a df range.
STRATA = ((1, 1), (2, 9), (10, 99), (100, 999), (1000, 9999), (10000, 99999), (100000, 999999), (1000000, None))
LONGEST = 100


class Rng:
    """splitmix64: the same stream on every Python version."""

    def __init__(self, seed):
        self.state = seed & 0xFFFFFFFFFFFFFFFF

    def next(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & 0xFFFFFFFFFFFFFFFF
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & 0xFFFFFFFFFFFFFFFF
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & 0xFFFFFFFFFFFFFFFF
        return z ^ (z >> 31)

    def below(self, n):
        return self.next() % n


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def entry(query, shape, words):
    tags = [shape]
    if shape not in ("term", "phrase"):
        tags.append("global")
    if shape != "term":
        tags.append("%s:num_tokens_%d" % (shape, words))
    return {"query": query, "tags": tags}


def shapes(words, rng_term, index):
    """Every benchmark shape for one source query. `index` counts sources, for the alternating rules."""
    out = []
    n = len(words)
    out.append(entry(words[rng_term.below(n)], "term", 1))
    if n >= 2:
        out.append(entry(" ".join("+" + w for w in words), "intersection", n))
        out.append(entry('"%s"' % " ".join(words), "phrase", n))
        out.append(entry(" ".join(words), "union", n))
        head = words[:4]
        pos = index % 2
        out.append(entry(" ".join(("+" + w) if i == pos else w for i, w in enumerate(head)), "intersection_union", len(head)))
        head = words[:3]
        negated = min(1 + index % 2, len(head) - 1)
        required, rest = head[0], head[1 : 1 + negated]
        if required not in rest and len(set(rest)) == len(rest):
            out.append(entry(" ".join(["+" + required] + ["-" + w for w in rest]), "negated", 1 + len(rest)))
    return out


def write_set(rows, exclude, out):
    """Writes rows not seen in `exclude` or earlier in the set; returns per-category counts."""
    seen = set()
    for path in exclude:
        with open(path) as handle:
            seen.update(json.loads(line)["query"] for line in handle if line.strip())
    kept = []
    for row in rows:
        if row["query"] in seen:
            continue
        seen.add(row["query"])
        kept.append(row)
    with open(out, "w") as handle:
        for row in kept:
            handle.write(json.dumps(row) + "\n")
    return collections.Counter(row["tags"][0] for row in kept)


def cmd_candidates(args):
    questions = []
    seen = set()
    with open(args.raw) as handle:
        for line in handle:
            question = json.loads(line)["question"].strip()
            if question not in seen:
                seen.add(question)
                questions.append(question)
    total = len(questions)
    if args.sample and args.sample < total:
        rng = Rng(args.seed)
        picked = set()
        while len(picked) < args.sample:
            picked.add(rng.below(total))
        questions = [questions[i] for i in sorted(picked)]
    survivors = []
    for question in questions:
        if not LETTERS_ONLY.match(question):
            continue
        words = WHITESPACE.split(question.strip())
        if len(words) >= 2:
            survivors.append(words)
    with open(args.out, "w") as cand, open(args.phrases, "w") as phrases:
        for words in survivors:
            cand.write(json.dumps({"words": words}) + "\n")
            phrases.write('"%s"\n' % " ".join(words))
    print(json.dumps({"questions": total, "sampled": len(questions), "letters_only_two_terms": len(survivors)}))


def cmd_build_nq(args):
    with open(args.candidates) as handle:
        candidates = [json.loads(line)["words"] for line in handle]
    with open(args.counts) as handle:
        counts = [line.strip() for line in handle]
    if len(counts) != len(candidates):
        sys.exit("counts and candidates differ in length")
    rng = Rng(args.seed)
    rows = []
    hits = 0
    for words, count in zip(candidates, counts):
        if count == "ERR" or int(count) < 1:
            continue
        rows.extend(shapes(words, rng, hits))
        hits += 1
    counter = write_set(rows, args.exclude, args.out)
    print(json.dumps({"candidates": len(candidates), "with_phrase_hit": hits, "per_category": {c: counter[c] for c in CATEGORIES}}))


def load_vocab(path):
    terms = []
    with open(path) as handle:
        for line in handle:
            if line.startswith("#"):
                continue
            term, df = line.rstrip("\n").split("\t")
            terms.append((term, int(df)))
    return terms  # df descending


def strata_of(terms):
    strata = [[] for _ in STRATA]
    for rank, (term, df) in enumerate(terms):
        if rank < LONGEST:
            strata[-1].append(term)
            continue
        for i, (lo, hi) in enumerate(STRATA[:-1]):
            if df >= lo and (hi is None or df <= hi):
                strata[i].append(term)
                break
    return [s for s in strata if s]


def corpus_windows(path, stride, rng):
    """One random 2 to 4 word window from every `stride`-th document of the corpus: real phrases of the index."""
    windows = []
    with open(path) as handle:
        for number, line in enumerate(handle):
            if number % stride:
                continue
            words = json.loads(line)["text"].split()
            length = 2 + rng.below(3)
            if len(words) > length:
                start = rng.below(len(words) - length)
                windows.append(words[start : start + length])
    return windows


def cmd_build_random(args):
    terms = load_vocab(args.vocab)
    strata = strata_of(terms)
    rng = Rng(args.seed)
    def pick():
        stratum = strata[rng.below(len(strata))]
        return stratum[rng.below(len(stratum))]

    def length():
        draw = rng.below(10)
        return 2 if draw < 6 else 3 if draw < 9 else 4

    windows = corpus_windows(args.corpus, args.stride, rng)
    rows = []
    for index in range(args.sources):
        # Each category draws its own sources: random vocabulary terms, except phrases, which are real
        # adjacent words of the corpus so that a phrase has at least one hit, as the benchmark requires.
        rows.append(entry(pick(), "term", 1))
        words = [pick() for _ in range(length())]
        rows.extend(r for r in shapes(words, rng, index) if r["tags"][0] in ("intersection", "union", "intersection_union", "negated"))
        window = windows[rng.below(len(windows))]
        rows.append(entry('"%s"' % " ".join(window), "phrase", len(window)))
    counter = write_set(rows, args.exclude, args.out)
    print(json.dumps({"strata": len(strata), "windows": len(windows), "per_category": {c: counter[c] for c in CATEGORIES}}))


def cmd_build_sweep(args):
    strata = strata_of(load_vocab(args.vocab))
    rng = Rng(args.seed)
    rows = []
    for length in SWEEP_LENGTHS:
        for _ in range(args.per_length):
            words = set()
            while len(words) < length:
                stratum = strata[rng.below(len(strata))]
                words.add(stratum[rng.below(len(stratum))])
            ordered = sorted(words, key=lambda w: (rng.next(), w))
            rows.append(entry(" ".join(ordered), "union", length))
    counter = write_set(rows, [], args.out)
    print(json.dumps({"lengths": list(SWEEP_LENGTHS), "union": counter["union"]}))


def gutenberg_body(text):
    start = re.search(r"\*\*\* ?START OF (THE|THIS) PROJECT GUTENBERG EBOOK[^\n]*\n", text)
    end = re.search(r"\*\*\* ?END OF (THE|THIS) PROJECT GUTENBERG EBOOK", text)
    return text[start.end() if start else 0 : end.start() if end else len(text)]


def long_sentences(paths):
    """Sentences of 20 to 60 words after the corpus transform, in file order, without repeats."""
    seen = set()
    out = []
    for path in paths:
        with open(path, encoding="utf-8", errors="replace") as handle:
            body = gutenberg_body(handle.read())
        for sentence in SENTENCE_END.split(" ".join(body.split())):
            words = NON_LETTERS.sub(" ", sentence.lower()).split()
            if LONG_MIN <= len(words) <= LONG_MAX:
                query = " ".join(words)
                if query not in seen:
                    seen.add(query)
                    out.append(words)
    return out


def cmd_build_long(args):
    pool = long_sentences(args.books)
    rng = Rng(args.seed)
    order = list(range(len(pool)))
    for i in range(len(order) - 1, 0, -1):  # Fisher-Yates
        j = rng.below(i + 1)
        order[i], order[j] = order[j], order[i]
    rows = [entry(" ".join(pool[i]), "union", len(pool[i])) for i in order[: args.count]]
    counter = write_set(rows, args.exclude, args.out)
    print(json.dumps({"eligible_sentences": len(pool), "union": counter["union"]}))


def cmd_summary(args):
    for path in args.files:
        counter = collections.Counter()
        with open(path) as handle:
            for line in handle:
                counter[json.loads(line)["tags"][0]] += 1
        print(json.dumps({"file": path, "sha256": sha256(path), "per_category": {c: counter[c] for c in CATEGORIES}}))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("candidates")
    p.add_argument("--raw", required=True)
    p.add_argument("--seed", type=int, required=True)
    p.add_argument("--sample", type=int, default=0)
    p.add_argument("--out", required=True)
    p.add_argument("--phrases", required=True)
    p.set_defaults(run=cmd_candidates)
    p = sub.add_parser("build-nq")
    p.add_argument("--candidates", required=True)
    p.add_argument("--counts", required=True)
    p.add_argument("--seed", type=int, required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--exclude", nargs="*", default=[])
    p.set_defaults(run=cmd_build_nq)
    p = sub.add_parser("build-random")
    p.add_argument("--vocab", required=True)
    p.add_argument("--corpus", required=True)
    p.add_argument("--seed", type=int, required=True)
    p.add_argument("--sources", type=int, default=400)
    p.add_argument("--stride", type=int, default=25)
    p.add_argument("--out", required=True)
    p.add_argument("--exclude", nargs="*", default=[])
    p.set_defaults(run=cmd_build_random)
    p = sub.add_parser("build-sweep")
    p.add_argument("--vocab", required=True)
    p.add_argument("--seed", type=int, required=True)
    p.add_argument("--per-length", type=int, default=60)
    p.add_argument("--out", required=True)
    p.set_defaults(run=cmd_build_sweep)
    p = sub.add_parser("build-long")
    p.add_argument("--books", nargs="+", required=True)
    p.add_argument("--seed", type=int, required=True)
    p.add_argument("--count", type=int, default=400)
    p.add_argument("--out", required=True)
    p.add_argument("--exclude", nargs="*", default=[])
    p.set_defaults(run=cmd_build_long)
    p = sub.add_parser("summary")
    p.add_argument("files", nargs="+")
    p.set_defaults(run=cmd_summary)
    args = parser.parse_args()
    args.run(args)


if __name__ == "__main__":
    main()
