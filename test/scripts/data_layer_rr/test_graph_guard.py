"""Imported graph algorithms must not become an unobserved Python fallback."""
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from internal_dead_code_port import graph_algorithm_codes


class GraphGuardTests(unittest.TestCase):
    def test_imported_algorithms_and_their_closures_are_guarded(self):
        class Graph:
            def __init__(self):self.roots=set()
            def reachable(self):
                def worker():return self.roots
                return worker()
        codes=graph_algorithm_codes(SimpleNamespace(SymbolGraph=Graph))
        self.assertNotIn(Graph.__init__.__code__,codes)
        self.assertIn(Graph.reachable.__code__,codes)
        nested=next(c for c in Graph.reachable.__code__.co_consts if hasattr(c,'co_code'))
        self.assertIn(nested,codes)
        self.assertFalse(codes[nested][2])

    def test_profile_rejects_a_graph_algorithm_call(self):
        class Graph:
            def reachable(self):return set()
        codes=graph_algorithm_codes(SimpleNamespace(SymbolGraph=Graph))
        def guard(frame,event,arg):
            if event=='call' and frame.f_code in codes:raise AssertionError('Python graph fallback')
        try:
            sys.setprofile(guard)
            with self.assertRaisesRegex(AssertionError,'Python graph fallback'):Graph().reachable()
        finally:sys.setprofile(None)
