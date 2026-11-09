"""Native signing keeps key material outside rule and utility request payloads."""
import json
from pathlib import Path
import sys
import types
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import migration
import replay


class SigningTransportTests(unittest.TestCase):
    def test_unknown_fields_and_nonstrings_never_reach_a_request_file(self):
        for requests in [[dict(id='a',key_env='fixture',message='public',key='synthetic')],
                         [dict(id='a',key_env=123,message='public')],{'id':'a'}]:
            with self.subTest(requests=requests),patch.object(replay.tempfile,'TemporaryDirectory') as temporary:
                with self.assertRaisesRegex(ValueError,'only id, key_env, and message'):
                    replay.sign_native(Path('/engine'),requests)
                temporary.assert_not_called()

    def test_artifact_audit_precedes_signing_and_request_file_creation(self):
        with patch.object(migration,'read_json',return_value=dict(engineSource={})), \
             patch.object(migration,'verify_source_cohort'), \
             patch.object(migration,'verify_engine_artifact',side_effect=ValueError('stale binary')), \
             patch.object(replay.tempfile,'TemporaryDirectory') as temporary,patch.object(replay.subprocess,'run') as run:
            with self.assertRaisesRegex(ValueError,'stale binary'):
                replay.sign_native(Path('/engine'),[dict(id='a',key_env='fixture',message='public')])
            temporary.assert_not_called();run.assert_not_called()

    def test_key_environment_is_never_serialized_and_batch_order_is_preserved(self):
        requests=[dict(id=i,key_env='fixture',message='public') for i in ['a','b']]
        replies=[dict(id=i,status='signed',signature='public signature',detail='') for i in ['a','b']]
        environment={'fixture':'synthetic-only'}
        def execute(command,**kwargs):
            text=Path(command[-1]).read_text()
            self.assertEqual(requests,json.loads(text));self.assertNotIn('synthetic-only',text)
            self.assertIs(environment,kwargs['env'])
            return types.SimpleNamespace(returncode=0,stdout=json.dumps(replies),stderr='')
        with patch.object(migration,'read_json',return_value=dict(engineSource={})), \
             patch.object(migration,'verify_source_cohort'),patch.object(migration,'verify_engine_artifact'), \
             patch.object(replay.subprocess,'run',side_effect=execute):
            self.assertEqual(replies,replay.sign_native(Path('/engine'),requests,environment))
            replies.reverse()
            with self.assertRaisesRegex(ValueError,'lost or reordered'):
                replay.sign_native(Path('/engine'),requests,environment)


if __name__=='__main__':unittest.main()
