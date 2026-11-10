"""Preserve supplied/disk identity, native read selection, and collector file groups."""
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from blanket_bypass_port import scan,run_documents


class BlanketTransportTests(unittest.TestCase):
    def test_supplied_text_and_disk_ast_keep_separate_documents(self):
        calls=[]
        class Backend:
            def native(self,check,entry,files,options=None):
                calls.append((entry,list(files),options))
                if entry=='_bypass_prepare_content':return [dict(code='NGB_READ',parameters=dict(ordinal=0,path='sql.py'))]
                return []
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);path=root/'sql.py';path.write_text('def execute_grammar(): pass\n')
            scan(Backend(),SimpleNamespace(SQL_CONNECTION_FILE='sql.py'),'scan_file_content',dict(root=root,path=path,source='execute_grammar'))
        self.assertEqual(2,len(calls));self.assertEqual(['source','disk'],calls[1][2]['bypassRoles'])
        self.assertEqual(['sql.py','sql.py'],calls[1][2]['bypassPaths'])
        self.assertEqual(['execute_grammar','def execute_grammar(): pass\n'],[f['source'] for f in calls[1][1]])

    def test_read_selection_precedes_opening_allowlisted_files(self):
        calls=[]
        class Backend:
            def native(self,check,entry,files,options=None):
                calls.append((entry,list(files),options))
                if entry=='_bypass_prepare_refs':
                    i=options['bypassCandidates'].index('api/a.py')
                    return [dict(code='NGB_READ',parameters=dict(ordinal=i,path='api/a.py'))]
                return []
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);allowed=root/'scripts/allowed.py';allowed.parent.mkdir();allowed.write_text('blocked')
            path=root/'api/a.py';path.parent.mkdir();path.write_text('_ALLOW_DIRECT_EXECUTION')
            original=Path.read_text
            def read(p,*a,**kw):
                if p==allowed:raise AssertionError('Allowlisted source was read')
                return original(p,*a,**kw)
            with patch.object(Path,'read_text',read):
                scan(Backend(),SimpleNamespace(SQL_CONNECTION_FILE='sql.py'),'check_no_bypass_references',dict(project_root=root))
        self.assertEqual(['_bypass_prepare_refs','check_no_bypass_references'],[c[0] for c in calls])
        self.assertEqual(1,len(calls[1][1]))

    def test_directory_target_is_present_without_becoming_source(self):
        calls=[]
        class Backend:
            def native(self,check,entry,files,options=None):calls.append((entry,files,options));return []
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/'sql.py').mkdir()
            self.assertEqual([],scan(Backend(),SimpleNamespace(SQL_CONNECTION_FILE='sql.py'),'check_no_public_grammar_methods',dict(project_root=root)))
        self.assertEqual(['false'],calls[0][2]['bypassIsFile']);self.assertEqual('true',calls[1][2]['bypassTargetExists'])
        self.assertEqual([],calls[1][1])

    def test_fact_rows_follow_native_file_then_event_positions(self):
        class Backend:
            def native(self,*args):
                return [dict(code='NGB_FACT',parameters=dict(file_order=f,phase=p,order0=o,order1=0,value_json=json.dumps(dict(message=m))))
                    for f,p,o,m in [(0,0,1,'sql-ref'),(1,0,3,'api-ref'),(0,1,2,'sql-method')]]
        rows=run_documents(Backend(),SimpleNamespace(SQL_CONNECTION_FILE='sql.py'),'_bypass_facts',[],{},{})
        self.assertEqual(['sql-ref','sql-method','api-ref'],[r['message'] for r in rows])
