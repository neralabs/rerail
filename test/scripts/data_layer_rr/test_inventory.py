"""The independent original inventory must reject incomplete or mixed collections."""
import copy
import importlib.util
import json
from pathlib import Path
import sys
import unittest

SCRIPTS = Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr'
sys.path.insert(0, str(SCRIPTS))
spec = importlib.util.spec_from_file_location('rr_original_inventory', SCRIPTS / 'inventory.py')
inventory = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inventory)


class InventoryTests(unittest.TestCase):
    def setUp(self):
        self.manifest = dict(sourcePin=inventory.PIN, scannedTestFiles=20, files={
            'tests/test_a.py': dict(detectors=['a'], integrationReferences=[], workflowReferences=[], sha256='a'),
            'tests/test_b.py': dict(detectors=['b'], integrationReferences=[], workflowReferences=[], sha256='b'),
        })
        self.shards = []
        for part, filename in enumerate(self.manifest['files']):
            identity = filename + '::test_original'
            self.shards.append(dict(part=part, parts=2, sourcePin=inventory.PIN,
                discoverySha256=inventory.sha(json.dumps(self.manifest, sort_keys=True).encode()),
                collectorSha256='collector', files={filename: dict(exitCode=0, errors=[], deselected=[],
                    before=[identity], cases={identity: dict(file=filename, assertSites=[dict(line=1, expression='original')])})}))

    def test_complete_collections_keep_every_assertion(self):
        result = inventory.assemble(self.manifest, self.shards)
        self.assertEqual(2, len(result['cases']))
        self.assertTrue(all(case['assertSites'][0]['expression'] == 'original' for case in result['cases'].values()))

    def test_missing_or_duplicate_shards_are_rejected(self):
        for shards in [self.shards[:1], [self.shards[0], self.shards[0]]]:
            with self.subTest(shards=shards), self.assertRaises(ValueError):
                inventory.assemble(self.manifest, shards)

    def test_omitted_module_is_rejected(self):
        self.shards[1]['files'] = {}
        with self.assertRaises(ValueError):
            inventory.assemble(self.manifest, self.shards)

    def test_deselected_failed_or_uncollected_case_is_rejected(self):
        for change in [dict(deselected=['lost']), dict(errors=['error']), dict(exitCode=1), dict(cases={})]:
            shards = copy.deepcopy(self.shards)
            shards[0]['files']['tests/test_a.py'].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                inventory.assemble(self.manifest, shards)

    def test_mixed_source_or_collector_is_rejected(self):
        for key in ['sourcePin', 'discoverySha256', 'collectorSha256']:
            shards = copy.deepcopy(self.shards)
            shards[1][key] = 'different'
            with self.subTest(key=key), self.assertRaises(ValueError):
                inventory.assemble(self.manifest, shards)

    def test_cases_cannot_claim_another_modules_assertions(self):
        value = self.shards[0]['files']['tests/test_a.py']
        old = value['before'][0]
        forged = 'tests/test_b.py::test_forged'
        value['before'] = [forged]
        value['cases'] = {forged: value['cases'][old]}
        with self.assertRaises(ValueError):
            inventory.assemble(self.manifest, self.shards)

    def test_import_aliases_and_dynamic_imports_are_discovered(self):
        owners = {'scripts.check_original': {'a'}}
        for text in ['from scripts import check_original as renamed',
                     'import scripts.check_original as renamed',
                     'importlib.import_module("scripts.check_original")']:
            with self.subTest(text=text):
                self.assertEqual(['a'], inventory.references(text, owners, ['a'])['detectors'])


if __name__ == '__main__':
    unittest.main()
