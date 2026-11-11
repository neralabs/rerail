"""Compare actual generated-index API returns, including traversal and alias edge cases."""
import ast
from pathlib import Path
from contract_runner import main
from dto import canonical
from generated_indexes_port import install

CHECK='generated_index_sources'
OWNER='impl/common/l7245_versioned_backends/generic_sparse.py'


def cases(_module):
    sources={
        'direct':"Index()\n",
        'clean':"x = 1\n",
        'required':"def _generated_index():\n return Index()\n",
        'valid':"def _generated_index(*, source: IndexSource):\n return Index(source=source)\n",
        'annotation':"def _generated_index(*, source: str):\n return Index(source=source)\n",
        'missing-annotation':"def _generated_index(*, source):\n return Index(source=source)\n",
        'quoted-annotation':"def _generated_index(*, source: 'IndexSource'):\n return Index(source=source)\n",
        'parenthesized-annotation':"def _generated_index(*, source: (IndexSource)):\n return Index(source=source)\n",
        'default':"def _generated_index(*, source: IndexSource=IndexSource.MODEL_DECLARED):\n return Index(source=source)\n",
        'bare-default':"def _generated_index(*, source: IndexSource=MODEL_DECLARED):\n return Index(source=source)\n",
        'positional-default':"def _generated_index(source: IndexSource=MODEL_DECLARED):\n return Index(source=source)\n",
        'positional-only':"def _generated_index(source: IndexSource, /):\n return Index(source=source)\n",
        'no-forwarding':"def _generated_index(source: IndexSource):\n return Index(source=other)\n",
        'async-helper':"async def _generated_index():\n return Index(source=source)\n",
        'from-alias':"from l1000_schema.schema_def import Index as Idx\nIdx()\n",
        'module-alias':"import l1000_schema.schema_def as sd\nsd.Index()\n",
        'implicit-module-alias':"import l1000_schema.schema_def\nschema_def.Index()\n",
        'alias-after-call':"Idx()\nfrom l1000_schema.schema_def import Index as Idx\nIdx()\n",
        'sticky-alias':"from l1000_schema.schema_def import Index as Idx\nIdx = other\nIdx()\n",
        'nested-alias':"def f():\n from l1000_schema.schema_def import Index as Idx\nIdx()\n",
        'attribute-receiver':"factory().Index()\nobj.Index()\nschema_def.Index()\n",
        'nested-def':"def _generated_index(source: IndexSource):\n def inner():\n  return Index()\n return Index(source=source)\n",
        'nested-class':"def _generated_index(source: IndexSource):\n class C:\n  field=Index(source=source)\n",
        'lambda':"def _generated_index(source: IndexSource):\n return lambda: Index(source=source)\n",
        'multiple':"def _generated_index(*, source: str=MODEL_DECLARED):\n Index()\n Index()\n",
        'no-trigger':"from l1000_schema.schema_def import Index as Idx\nIdx ()\n",
        'comments-trigger':"# Index(\nfrom l1000_schema.schema_def import Index as Idx\nIdx ()\n",
        'syntax-error':"Index(\n",
        'default-call':"def _generated_index(source: IndexSource=Index()):\n return Index(source=source)\n",
        'decorator-call':"@Index()\ndef _generated_index(source: IndexSource):\n return Index(source=source)\n",
        'decorator-alias':"@Idx()\ndef f():\n from l1000_schema.schema_def import Index as Idx\n return Index()\n",
        'header-alias-order':"@Idx()\ndef f(x=Idx()):\n from l1000_schema.schema_def import Index as Idx\n return Index()\n",
        'keyword-spread':"def _generated_index(source: IndexSource):\n return Index(**{'source':source})\n",
        'duplicate-keyword':"def _generated_index(source: IndexSource):\n return Index(source=other,source=source)\n",
        'unicode-call':"# Index(\nＩｎｄｅｘ()\n",
        'unicode-helper':"# _generated_index\ndef _ｇenerated_index(*, ｓource: ＩｎｄｅｘＳｏｕｒｃｅ):\n return Index(ｓource=ｓource)\n",
        'unicode-import':"from l1000_schema.schema_def import Ｉｎｄｅｘ as Ｉｄｘ\nＩｄｘ()\n# Index(\n",
    }
    result=[]
    for name,source in sources.items():
        for api in ['check_file','check_source','check_repo']:
            result.append(dict(id=name+':'+api,kind='identifier' if name.startswith('unicode') else 'scanner',api=api,source=source,path=OWNER))
    for name,values in [('missing',dict(missing=True)),('invalid-utf8',dict(rawBytes='ff')),
                        ('non-python',dict(path='input.txt')),('test-path',dict(path='tests/test_index.py')),
                        ('colon-path',dict(path='a: b/generic_sparse.py'))]:
        result.append(dict(dict(id=name,kind='scanner',api='check_file',source='Index()\n',path=OWNER),**values))
    result.append(dict(id='colon-multiline',kind='scanner',api='check_file',path='a: b/generic_sparse.py',
        source='def _generated_index(*, source: str=MODEL_DECLARED):\n Index()\n'))
    return result


def exercise(module,case,scratch):
    root=scratch/'project';root.mkdir()
    path=root/case['path'];path.parent.mkdir(parents=True,exist_ok=True)
    if not case.get('missing'):
        path.write_bytes(bytes.fromhex(case['rawBytes']) if 'rawBytes' in case else case['source'].encode())
    try:
        if case['api']=='check_repo':value=module.check_repo(root)
        elif case['api']=='check_source':value=module.check_source(path,'Index() # deliberately different supplied text',ast.parse('different()'))
        else:value=module.check_file(path)
    except Exception as error:
        return dict(error=type(error).__name__,args=canonical(error.args,scratch))
    # Rendered source findings can contain the caller path twice after a parse
    # failure. Normalize only this run's random temporary root at every occurrence.
    return [v.replace(scratch.as_posix()+'/', '') for v in value]


if __name__=='__main__':
    main(CHECK,'rerail.generated-indexes-contract/v1',cases,exercise,install,
         ['generated_indexes_contract.py','generated_indexes_port.py'])
