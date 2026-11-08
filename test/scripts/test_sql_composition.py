"""The comparison must reject lost positive rows and incomplete executions."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "composition", Path(__file__).resolve().parents[2] / "scripts/profiling/composition.py"
)
LAB = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(LAB)


class CompositionParityTests(unittest.TestCase):
    def args(self, directory):
        return SimpleNamespace(output=str(directory / "results"), binary="unused-binary",
                               capture="unused-capture", experiment="demand", files="3",
                               depth=4, start_trial=0, trials=3)

    def test_lost_positive_multiplicity_aborts_the_comparison(self):
        def execute(command, **_):
            # Both paths find the same identity, but one loses a repeated answer.
            rows = [[1, 3, 4, 5]] * (2 if command[4] == "eager" else 1)
            Path(command[-1]).write_text(json.dumps({"rows": rows}))

        with tempfile.TemporaryDirectory() as tmp, patch.object(LAB.subprocess, "run", execute):
            directory = Path(tmp)
            with self.assertRaisesRegex(RuntimeError, "exact multiset parity failed"):
                LAB.run(self.args(directory))
            self.assertFalse((directory / "results/summary.json").exists())

    def test_failed_kernel_cannot_produce_a_success_summary(self):
        failure = subprocess.CalledProcessError(1, "composition")
        with tempfile.TemporaryDirectory() as tmp, patch.object(LAB.subprocess, "run", side_effect=failure):
            directory = Path(tmp)
            with self.assertRaises(subprocess.CalledProcessError):
                LAB.run(self.args(directory))
            self.assertFalse((directory / "results/summary.json").exists())


if __name__ == "__main__":
    unittest.main()
