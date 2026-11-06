"""The cumulative CPU receipt must include waited-for grandchildren."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

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
            for space in report['filesystems'].values():
                self.assertGreater(space['samples'], 0)
                self.assertLessEqual(space['minimumFreeBytes'], space['initialFreeBytes'])
                self.assertLessEqual(space['minimumFreeBytes'], space['finalFreeBytes'])

    def test_disk_pressure_between_endpoints_is_retained(self):
        readings = [SimpleNamespace(total=1000, free=free) for free in [100, 20, 90]]
        with patch.object(module.shutil, 'disk_usage', side_effect=readings):
            space = module.DiskSpace(Path('/test-volume'))
            space.sample(); space.sample()
        self.assertEqual(space.report(), dict(path='/test-volume', totalBytes=1000,
            initialFreeBytes=100, minimumFreeBytes=20, finalFreeBytes=90, samples=3, errors=[]))


if __name__ == '__main__':
    unittest.main()
