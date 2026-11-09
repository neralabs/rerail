"""Source-input cases cover every removed name and the collector's stack boundaries."""
CALLS=['_execute_grammar','_execute_sql_or_schema_statement','_execute_sql_or_schema_statement_async',
       '_execute_template_with_params','_execute_template_with_params_async','execute_grammar','execute_sql',
       'fetchall_grammar','fetchall_raw','fetchall_raw_async','fetchone_grammar','fetchone_raw','fetchone_raw_async',
       'nonportably_execute_sql','raw_execute','raw_execute_async','raw_execute_fetch','raw_execute_fetch_async']
DEFS=CALLS+['_execute_grammar_async','_is_schema_ddl_statement']
MIXED=['_execute_sql_or_schema_statement','_execute_sql_or_schema_statement_async','_is_schema_ddl_statement',
       'compatibility router','arbitrary grammar router']


def case(label,source,entry='_check_ast',path='sample.py',compatibility=None,**arguments):
    values=dict(root={'path':'.'},**arguments)
    if entry!='context':values['path']={'path':path}
    if compatibility is not None:values['compatibility']=compatibility
    return dict(id='raw-execute-'+label,check='raw_execute_regression',entry=entry,
                files=[dict(path=path,source=source)],arguments=values)


def cases():
    result=[]
    for name in CALLS:
        result.append(case('call-'+name,f'def caller(conn):\n conn.{name}()\n'))
        result.append(case('bare-'+name,f'def caller():\n {name}()\n'))
    for name in DEFS:
        for prefix in ['', 'async ']:
            result.append(case(('async-' if prefix else '')+'definition-'+name,f'{prefix}def {name}():\n pass\n'))
    for i,token in enumerate(MIXED):result.append(case('text-'+str(i),'# '+token+'\n','_check_text'))
    result.append(case('text-all','\n'.join(MIXED),'_check_text'))
    result.append(case('text-missing-file','','_check_text',text='arbitrary grammar router',path='empty.py'))
    bodies={
        'raise':'raise RuntimeError("removed")',
        'helper':'conn._raise_removed()',
        'bare-helper':'_raise_removed()',
        'banned-then-raise':'conn.raw_execute(); raise RuntimeError()',
        'helper-and-banned':'conn._raise_removed(); conn.raw_execute()',
        'nested-raise':'def inner():\n  raise RuntimeError()',
        'nested-banned':'def inner():\n  conn.raw_execute()',
        'nested-banned-outer-raise':'def inner():\n  conn.raw_execute()\n raise RuntimeError()',
        'lambda-helper':'callback = lambda: conn._raise_removed()',
        'class-raise':'class Nested:\n  raise RuntimeError()',
        'class-method-raise':'class Nested:\n  def method(self):\n   raise RuntimeError()',
        'class-decorator-helper':'@conn._raise_removed()\n class Nested: pass',
        'function-decorator-helper':'@conn._raise_removed()\n def nested(): pass',
        'default-helper':'def nested(x=conn._raise_removed()): pass',
    }
    for label,body in bodies.items():
        source='def raw_execute(conn):\n '+body.replace('\n','\n ')+'\n'
        for allowed in [False,True]:result.append(case('stub-'+label+'-'+str(allowed),source,compatibility=['sample.py'] if allowed else []))
    result.extend([
        case('nested-exit-order','def raw_execute(conn):\n conn.raw_execute()\n def raw_execute_async():\n  conn.raw_execute_async()\n conn.execute_sql()\nconn.fetchall_raw()\n'),
        case('same-line-multiplicity','conn.raw_execute(); conn.raw_execute(); conn.execute_sql()\n'),
        case('decorator-order','@conn.raw_execute()\ndef raw_execute(x=conn.execute_sql()) -> conn.fetchall_raw():\n conn.fetchone_raw()\n'),
        case('class-body-order','class C(conn.raw_execute()):\n @conn.execute_sql()\n def raw_execute(self):\n  conn.fetchall_raw()\n'),
        case('trigger-provided-text-disables','conn.raw_execute()\n',text='nothing relevant'),
        case('trigger-provided-text-does-not-replace-disk','value=1\n',text='conn.raw_execute()'),
        case('supplied-tree-is-ignored','conn.raw_execute()\n',tree={'pythonAst':'pass','mode':'exec'}),
        case('invalid-source','conn.raw_execute(\n'),
        case('unicode-normalized-attribute-gate','conn.ｒａｗ＿ｅｘｅｃｕｔｅ()\n'),
        case('unicode-normalized-attribute-enabled','# raw_execute\nconn.ｒａｗ＿ｅｘｅｃｕｔｅ()\n'),
        case('query-materializer','class WidgetQuery:\n def execute_raw(self): return []\n async def execute_raw_async(self): return []\n'),
        case('explicit-tests-file','conn.raw_execute()\n',path='tests/sample.py'),
        case('explicit-hidden-file','conn.raw_execute()\n',path='tmp/sample.py'),
    ])
    for raised in [False,True]:
        for helper in [False,True]:
            for banned in [False,True]:
                result.append(case(f'context-{raised}-{helper}-{banned}','',entry='context',name='raw_execute',line=1,
                                   class_stack={'tuple':['Conn']},has_raise=raised,has_raise_helper=helper,has_banned_call=banned))
    for name in ['execute_raw','execute_raw_async','raw_execute','other']:
        for classes in [[],['WidgetQuery'],['OuterQuery','Plain'],['Outer','Inner_CachedQuery']]:
            result.append(dict(id='raw-execute-materializer-'+name+'-'+str(len(result)),check='raw_execute_regression',
                               entry='_is_query_raw_materializer',files=[],arguments=dict(root={'path':'.'},name=name,class_stack={'tuple':classes})))
    for entry in ['_facts_for_file','_violation_strings_for_file','scan_file_content']:
        for label,source in [('mixed','name="compatibility router"\nconn.raw_execute(); conn.raw_execute()\n'),('invalid','conn.raw_execute(\n')]:
            args=dict(text=source) if entry=='scan_file_content' else dict(include_text=True)
            result.append(case(entry+'-'+label,source,entry=entry,**args))
    for path in ['sample.py','impl/sample.py','tests/sample.py','test/sample.py','conform/sample.py','scripts/benchmark_extra.py',
                 'scripts/lint_raw_execute_regression.py','impl/.ruff_cache/sample.py','impl/node_modules/sample.py','impl/browser_build/sample.py']:
        result.append(case('excluded-'+path,'conn.raw_execute()\n',entry='_is_excluded',path=path))
        result.append(case('scan-'+path,'conn.raw_execute()\n',entry='scan_file_content',path=path,text='raw_execute'))
    for value in ['plain','file:message','file:12: message: detail','file:000: message','file:١٢: msg','file:²: msg',
                  'file:not-a-number: ignored','file:: ignored',':3: \u2003hello\u001c','a:-2:message']:
        result.append(dict(id='raw-execute-fact-'+str(len(result)),check='raw_execute_regression',entry='_violation_text_to_fact',
                           files=[],arguments=dict(root={'path':'.'},text=value)))
    for path in ['a:b.py','a:2:b.py','a:²:b.py']:
        result.append(case('colon-path-'+path,'conn.raw_execute()\n',path=path))
    return result


CASES=cases()


def reporter_cases():
    paths=['impl/a.py','data_layer/a.py','_internal/a.py','l1234_ok/a.py','l١٢٣٤_unicode/a.py','l1234_\nno/a.py',
           'api/a.py','other/a.py','l123_short/a.py','l1234_/a.py','impl/tests/a.py','impl/test/a.py','impl/conform/a.py',
           'impl/tmp/a.py','impl/.git/a.py','impl/.venv/a.py','impl/venv/a.py','impl/__pycache__/a.py',
           'impl/.ruff_cache/a.py','impl/.pytest_cache/a.py','impl/node_modules/a.py','impl/browser_build/a.py',
           'impl/a:b.py','impl/a:2:b.py','impl/a:²:b.py']
    rows=[dict(id='raw-reporter-'+str(i),check='raw_execute_regression',files=[dict(path=path,source='conn.raw_execute(); conn.raw_execute()\n')]) for i,path in enumerate(paths)]
    for name,source,path in [
        ('mixed-all','\n'.join('# '+s for s in MIXED)+'\nconn.raw_execute()\n','impl/mixed.py'),
        ('definitions','\n'.join('def '+n+'(): pass' for n in DEFS)+'\n','impl/defs.py'),
        ('compatibility-raise','def raw_execute(conn): raise RuntimeError()\n','impl/sqlite/connection.py'),
        ('compatibility-wrapper','def raw_execute(conn):\n conn.raw_execute()\n raise RuntimeError()\n','impl/sqlite/connection.py'),
        ('parse-failure','# compatibility router\nconn.raw_execute(\n','impl/bad.py'),
        ('clean','class WidgetQuery:\n def execute_raw(self): return []\n','impl/query.py')]:
        rows.append(dict(id='raw-reporter-'+name,check='raw_execute_regression',files=[dict(path=path,source=source)]))
    return rows


REPORTER_CASES=reporter_cases()
