# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0

import re
import unittest

import longmemeval_compact_context as K


def turn(session, index, role, text, time=1_675_000_000_000):
    return {"sourceId": f"{session}-{index}", "sessionId": session, "turnIndex": index, "role": role,
            "observationTime": time + index, "text": text}


class CompactContextTest(unittest.TestCase):
    def test_squeeze_keeps_every_number_and_name(self):
        text = ("Sure! I'm happy to help.\n\n## Plan\n1. **Visit** the Louvre Museum on 3/14 at 10:30.\n"
                "- Budget: $1,250 for 2 adults.\nI hope this helps!")
        squeezed = K.squeeze(text)
        for kept in ("Louvre Museum", "3/14", "10:30", "$1,250", "2 adults", "Visit"):
            self.assertIn(kept, squeezed)
        for dropped in ("Sure", "happy to help", "hope this helps", "**", "##", "\n"):
            self.assertNotIn(dropped, squeezed)
        self.assertLess(K.tokens(squeezed), K.tokens(text))
        user = "Thanks to my sister I finally read The Nightingale."
        self.assertEqual(K.squeeze(user, articles=False, niceties=False), user)

    def test_legend_round_trips_names(self):
        text = "New York City then New York City and New York City; Paris"
        defined = K.legend(text)
        header, body = defined.split("\n", 1)
        names = dict(item.split("=", 1) for item in header.removeprefix("defs:").split("|"))
        self.assertEqual(re.sub(r"\$\d+", lambda m: names[m.group(0)], body), text)
        self.assertEqual(K.legend("Paris once"), "Paris once")

    def test_fill_keeps_the_base_and_never_exceeds_the_budget(self):
        sources = [turn("s1", 0, "user", "I adopted a cat named Miso."),
                   turn("s2", 0, "user", "I adopted a second cat, Tofu, last week."),
                   turn("s3", 0, "user", " ".join(["filler"] * 400))]
        available = {s["sourceId"]: s for s in sources}
        render = K.renderer(K.squeeze, K.squeeze)
        base = sources[:1]
        for budget in (K.tokens(render(base)), K.tokens(render(sources[:2])) - 1,
                       K.tokens(render(sources[:2])), K.tokens(render(sources))):
            chosen = K.fill(list(available), available, budget, render, base, lambda sid: [available[sid]])
            self.assertLessEqual({s["sourceId"] for s in base}, {s["sourceId"] for s in chosen})
            self.assertLessEqual(K.tokens(render(chosen)), budget)
        self.assertEqual(len(K.fill(list(available), available, K.tokens(render(sources)), render, base,
                                    lambda sid: [available[sid]])), 3)

    def test_round_robin_interleaves_sessions_in_rank_order(self):
        available = {sid: {"sessionId": sid[0]} for sid in ("a1", "a2", "b1", "a3", "c1", "b2")}
        self.assertEqual(K.round_robin(["a1", "a2", "b1", "a3", "c1", "b2"], available),
                         ["a1", "b1", "c1", "a2", "b2", "a3"])

    def test_symbolic_marks_elided_turns_and_orders_sessions_by_time(self):
        out = K.symbolic([turn("late", 0, "user", "second", time=2_000_000_000_000),
                          turn("early", 4, "assistant", "reply"), turn("early", 1, "user", "first")])
        self.assertLess(out.index("first"), out.index("second"))
        self.assertIn("U: first\n...\nA: reply", out)


class CompactHistoryTest(unittest.TestCase):
    def entry(self, question):
        user = "I finished The Nightingale by Kristin Hannah, really loved it."
        reply = "Sure! That's a great book. **The Nightingale** follows two sisters in France."
        return {"question": question, "question_date": "2023/02/07 (Tue) 09:09",
                "haystack_session_ids": ["s1", "s2"], "haystack_dates": ["2023/02/01 (Wed) 10:00", "2023/01/03 (Tue) 08:00"],
                "haystack_sessions": [[{"role": "user", "content": user}, {"role": "assistant", "content": reply},
                                       {"role": "user", "content": "Thanks."}, {"role": "assistant", "content": "Bye!"}],
                                      [{"role": "user", "content": "I started a new book."},
                                       {"role": "assistant", "content": "Enjoy it!"}]]}

    def test_compact_keeps_the_official_rounds_and_literal_user_text(self):
        import longmemeval_official_qa as Q

        entry = self.entry("Which book did I finish a week ago?")
        ranked = ["s1_1", "s2_1", "s1_3"]
        for topk in (1, 2, 3):
            compact = Q.compact_history(entry, ranked, topk)
            official = Q.official_prompt(entry, ranked, topk)
            for text in ("I finished The Nightingale by Kristin Hannah, really loved it.", "I started a new book.", "Thanks."):
                self.assertEqual(text in compact, text in official, (topk, text))
            self.assertLess(K.tokens(compact), K.tokens(official))
        compact = Q.compact_history(entry, ranked, 3)
        self.assertLess(compact.index("2023/01/03"), compact.index("2023/02/01"))
        self.assertNotIn("Sure!", compact)
        self.assertNotIn("**", compact)
        self.assertIn("**The Nightingale**", Q.compact_history(self.entry("What did you say about the book?"), ranked, 3))


if __name__ == "__main__":
    unittest.main()
