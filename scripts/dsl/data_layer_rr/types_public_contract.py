"""Record type-safety scanner windows, disk/content gates, paths and overrides."""
import contextlib
from pathlib import Path
from contract_runner import main
from dto import canonical
from types_cli_contract import CHECK,BAD,install


def cases(_module):
    rows=[]
    shapes=dict(positive=BAD,clean='pass\n',invalid='def broken(: # str | Expression\n',
        all_patterns='x: str | Expression | str | Field\nx: Expression[Any, Any]\nx = dict(_lt=1, _gt=2)\nx.filter_gt(1).filter_lt(2)\n',
        signature_body='def where():\n x: str | Expression\n',
        order_body='def order_by():\n column: str\n',
        order_multiline='def order_by(\n column: str,\n):\n pass\n',
        order_limit='def order_by(\n'+''.join(' # spacing\n' for _ in range(12))+' column: str,\n):\n pass\n',
        nested='def f():\n def select(columns: str):\n  pass\n',
        async_where='async def where(expression: str):\n pass\n',
        nfkc='def ｏｒｄｅｒ＿ｂｙ(column: str):\n pass\n',
        splitlines='x = "str | Expression\\n"\n# str | Field\u2028# Expression[Any, Any]\n',
        comment='# str | Expression\n',
        unicode_boundary='éstr | Expression\nstr\u2003|\u2003Expression\n')
    for entry in ['check_file_for_patterns','check_file_content_for_patterns','check_function_signatures','check_function_signature_content']:
        rows.extend(dict(kind='scan',id=entry+'-'+name,entry=entry,disk=source,content=source) for name,source in shapes.items())
        for path in ['tests/a.py','test/a.py','other/a.py','.py','a.pyi','impl/query_spec.py']:
            rows.append(dict(kind='scan',id=entry+'-path-'+path,entry=entry,path=path,disk=BAD,content=BAD))
        rows.append(dict(kind='scan',id=entry+'-missing',entry=entry,disk=None,content=BAD))
        rows.append(dict(kind='scan',id=entry+'-binary',entry=entry,binary='ff',content=BAD))
    for i,(path,root) in enumerate([('/project/a.py','/'),('//server/a.py','//'),('a/../b.py','a'),('a.py','.'),('/project/a.py','//'),('//server/a.py','/'),('a.py','a.py')]):rows.append(dict(kind='normalize',id='normalize-'+str(i),path=path,root=root))
    rows.extend([dict(kind='scan',id='content-trigger-reads-clean-disk',entry='check_function_signature_content',disk='pass\n',content=BAD),dict(kind='scan',id='content-clean-gates-positive-disk',entry='check_function_signature_content',disk=BAD,content='pass\n'),dict(kind='scan',id='mutable-allowlist',entry='check_file_for_patterns',disk=BAD,override=True)])
    return rows


def exercise(module,case,scratch):
    root=scratch/'repo';root.mkdir();path=root/('impl/a.py' if case['kind']=='normalize' else case.get('path','impl/a.py'));path.parent.mkdir(parents=True,exist_ok=True)
    if case.get('binary'):path.write_bytes(bytes.fromhex(case['binary']))
    elif case.get('disk') is not None:path.write_text(case['disk'])
    try:
        if case['kind']=='normalize':value=module.normalize_path(Path(case['path']),Path(case['root']))
        else:
            original=module.ALLOWLIST
            if case.get('override'):module.ALLOWLIST={module.IssueCategory.STR_UNION_EXPRESSION:{'impl/a.py'}}
            try:
                args=[path,root]
                if 'content' in case['entry']:args.append(case['content'])
                value=getattr(module,case['entry'])(*args)
            finally:module.ALLOWLIST=original
        return dict(value=canonical(value,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


if __name__=='__main__':
    main(CHECK,'rerail.type-safety-public/v1',cases,exercise,install,['types_public_contract.py','types_cli_contract.py','types_port.py'])
