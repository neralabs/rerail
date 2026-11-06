"""The cumulative CPU receipt must include waited-for grandchildren."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('complete_measurement', 'scripts/profiling/data_layer/measure.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class CompleteMeasurement(unittest.TestCase):
    def test_child_cpu_and_exit_status_are_preserved(self):
        child = 'import time; start=time.process_time();\nwhile time.process_time()-start < .12: pass'
        parent = 'import subprocess,sys; subprocess.run([sys.executable,"-c",' + repr(child) + '],check=True); raise SystemExit(17)'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'receipt.json'
            self.assertEqual(module.measure([sys.executable, '-c', parent], output), 17)
            report = json.loads(output.read_text())
            self.assertGreaterEqual(report['cpuSeconds'], .12)
            self.assertGreaterEqual(report['wallSeconds'], .12)
            self.assertEqual(report['exitCode'], 17)
            self.assertGreater(report['maxSingleProcessRssBytes'], 0)


if __name__ == '__main__':
    unittest.main()
