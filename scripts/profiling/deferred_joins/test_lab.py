"""Regressions for the controls and the completion cache."""

import itertools
import unittest

from lab import connect
from strategies import Strategy
from witness_bounds import bounds


class WitnessBoundsTests(unittest.TestCase):
    def test_interval_contains_every_small_join(self):
        populations = list(itertools.product([0, 1, 3], repeat=3))
        for left, right in itertools.product(populations, repeat=2):
            expected = sum(a * b for a, b in zip(left, right))
            for top in (0, 1, 3):
                result = bounds(
                    ((k, n) for k, n in enumerate(left) if n),
                    ((k, n) for k, n in enumerate(right) if n),
                    top,
                )
                self.assertLessEqual(result["lower_witnesses"], expected)
                self.assertGreaterEqual(result["upper_witnesses"], expected)
                self.assertEqual(result["oracle_witnesses"], expected)
                if top == 3:
                    self.assertEqual(
                        result["lower_witnesses"], result["upper_witnesses"]
                    )

    def test_same_degrees_different_hot_key(self):
        big = 1 << 40
        left = [(1, big), (2, 1)]
        for right, expected in [(left, big * big + 1), ([(1, 1), (2, big)], 2 * big)]:
            result = bounds(left, right, 1)
            self.assertLessEqual(result["lower_witnesses"], expected)
            self.assertEqual(result["upper_witnesses"], expected)


class LabTests(unittest.TestCase):
    def setUp(self):
        self.db = connect("sparse", None)
        self.addCleanup(self.db.close)

    def test_membership_control_uses_both_key_columns(self):
        Strategy(self.db, "eager", True, 3).run([])
        plan = self.db.execute("""EXPLAIN QUERY PLAN SELECT q.i,w.a,w.b
            FROM asked q CROSS JOIN wide w WHERE w.a=q.a AND w.b=q.b""")
        self.assertTrue(
            any("SEARCH w" in step and step.count("=?") == 2 for *_, step in plan)
        )

    def test_completed_miss_eviction_and_oversized_result(self):
        plan = Strategy(self.db, "tabled", True, 2)
        miss = (0, 999)
        hit = (0, 0)
        plan.run([miss])
        answers, stats = plan.run([miss, miss, hit])
        self.assertEqual((stats["distinct_keys"], stats["negative_hits"]), (2, 1))
        self.assertEqual(answers[hit], [hit])
        _, stats = plan.run([miss])
        self.assertEqual(stats["cache_hits"], 0)
        broad, stats = plan.run([(0, None)])
        self.assertGreater(len(broad[(0, None)]), plan.limit)
        self.assertNotIn((0, None), plan.cache)
        self.assertLessEqual(stats["cache_weight"], plan.limit)

    def test_negative_answer_expires_after_insert(self):
        plan = Strategy(self.db, "tabled", True, 20)
        key = (0, 999)
        self.assertFalse(plan.run([key])[0][key])
        self.db.execute("INSERT INTO r VALUES (999,1)")
        plan.invalidate()
        answers, stats = plan.run([key])
        self.assertEqual(answers[key], [key])
        self.assertEqual(stats["cache_hits"], 0)


if __name__ == "__main__":
    unittest.main()
