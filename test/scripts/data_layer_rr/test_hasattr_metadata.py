"""The async extractor metadata and fact DTOs must come from native outputs."""
from dataclasses import dataclass
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))


class HasattrMetadataTests(unittest.TestCase):
    def test_native_metadata_and_fact_dto_are_decoded_without_source_helpers(self):
        from hasattr_port import install
        @dataclass(frozen=True)
        class Spec:
            extractor_id:str
            source_triggers:tuple=()
            source_trigger_groups:tuple=()
            project_input_facts:tuple=()
        class Backend:
            def replace(self,owner,name,value):setattr(owner,name,value)
            def native(self,check,entry,files,options=None):
                if entry=='_async_configuration':
                    return [dict(code='ASYNC_CONFIG',parameters=dict(value=json.dumps(dict(extractor_id='native',source_triggers=['hasattr'],source_trigger_groups=[['_async']],project_input_facts=[]))))]
                return [dict(code='ASYNC_FACT',parameters=dict(line=4,kind_order=0,value_json='{"line":4,"pattern_type":"hasattr","method_name":"fetch_async"}'))]
        module=SimpleNamespace(HASATTR_ASYNC_EXTRACTOR=Spec('old'),_extract_hasattr_async_facts=None)
        install(Backend(),module)
        self.assertEqual(Spec('native',('hasattr',),(('_async',),),()),module.HASATTR_ASYNC_EXTRACTOR)
        self.assertEqual([dict(line=4,pattern_type='hasattr',method_name='fetch_async')],module._extract_hasattr_async_facts(Path('a.py'),'source'))
