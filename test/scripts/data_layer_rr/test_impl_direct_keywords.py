"""The compatibility adapter accepts the original public keyword argument names."""
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from impl_direct_execution_port import install


class Backend:
    def __init__(self):self.calls=[]
    def replace(self,module,name,value):setattr(module,name,value)
    def native(self,check,entry,files,options):
        self.calls.append(dict(check=check,entry=entry,files=files,options=options))
        if entry=='configuration':
            settings={'allowed_file':['impl/allowed.py'],'allowed_dir_pattern':['impl/tests/'],'direct_pattern':['pattern'],'fts_pattern':['pattern'],'signature':['signed']}
            return [dict(code='IDE_SETTING',parameters=dict(setting=k,ordinal=i,value=v)) for k,values in settings.items() for i,v in enumerate(values)]
        if entry=='is_allowed_file':return [dict(code='IDE_BOOL',parameters=dict(value=0))]
        return []


class Keywords(unittest.TestCase):
    def adapter(self):
        backend=Backend();module=SimpleNamespace(ALLOWED_FILES={'impl/allowed.py'},PROJECT_ROOT=Path('/project'))
        install(backend,module);return backend,module

    def test_allowlist_filepath_keyword(self):
        backend,module=self.adapter()
        self.assertFalse(module.is_allowed_file(filepath=Path('impl/a.py')))
        self.assertEqual(backend.calls[-1]['options']['argumentPath'],'impl/a.py')

    def check_scan(self,name):
        backend,module=self.adapter()
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary)/'input.py';path.write_text('obj._fetchone()\n')
            self.assertEqual(getattr(module,name)(filepath=path),[])
            call=backend.calls[-1]
            self.assertEqual(call['entry'],name)
            self.assertEqual(call['files'],[dict(path='input.py',source='obj._fetchone()\n')])
            self.assertEqual(call['options']['argumentPath'],str(path))

    def test_scan_file_filepath_keyword(self):self.check_scan('scan_file')
    def test_fts_scan_filepath_keyword(self):self.check_scan('scan_fts_manager_direct_execution')
