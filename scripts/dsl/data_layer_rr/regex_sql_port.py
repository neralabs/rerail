import ast
from pathlib import Path
from ast_transport import visit_columns

"""Transport regex helper arguments; native relations own classification."""


def graph(suite,entry,tree,path):
    options=visit_columns(tree);nodes=[];pending=[tree]
    while pending:
        node=pending.pop();nodes.append(node);pending.extend(reversed(list(ast.iter_child_nodes(node))))
    options['astConstantText']=[str(n.value) if isinstance(n,ast.Constant) else '' for n in nodes]
    options['astDepths']=[]
    for parent in options['astParents']:
        options['astDepths'].append(str(0 if parent=='-1' else int(options['astDepths'][int(parent)])+1))
    options['originalPath']=str(path)
    return suite.native('regex_sql_parsing',entry,[],options)


def violations(module,rows,breadth=True):
    rows=[r for r in rows if r['code']=='RSP_FACT']
    rows=sorted(rows,key=lambda r:((r['parameters']['depth'],) if breadth else ())+ (r['parameters']['ordinal'],))
    return [module.Violation(file=r['path'],line=r['line'],message=r['message'],severity='warning',category=r['parameters']['category'],checker='regex_sql_parsing') for r in rows]


def install(suite,module):
    callback_batch={}
    for name in ['_is_false_positive','_is_sql_syntax_pattern','_is_error_msg_pattern','_classify_pattern']:
        def factory(entry):
            def evaluate(pattern_str,func_name=None):
                options=dict(patternProbe=pattern_str)
                if func_name is not None:options['funcProbe']=func_name
                rows=suite.native('regex_sql_parsing',entry,[],options)
                if entry=='_classify_pattern':
                    return next((r['parameters']['value'] for r in rows if r['code']=='RSP_CATEGORY'),None)
                return any(r['code']=='RSP_TRUE' for r in rows)
            return evaluate
        suite.replace(module,name,factory(name))

    def extract(node):
        rows=graph(suite,'_extract_regex_string',node,Path('input.py'))
        return next((r['parameters']['value'] for r in rows if r['code']=='RSP_EXTRACT'),None)
    suite.replace(module,'_extract_regex_string',extract)
    for entry in ['check_file','_check_re_call','_check_re_compile']:
        def factory(name):
            def evaluate(checker,node,path,source):
                if name!='check_file' and id(node) in callback_batch:
                    return callback_batch[id(node)].get(name)
                rows=violations(module,graph(suite,name,node,path))
                return rows if name=='check_file' else next(iter(rows),None)
            if name=='check_file':
                def check_file(checker,tree,path,source):return evaluate(checker,tree,path,source)
                return check_file
            return evaluate
        suite.replace(module.RegexSqlParsingChecker,entry,factory(entry))
    metadata={r['parameters']['name']:r['parameters']['value'] for r in suite.native('regex_sql_parsing','metadata',[])}
    for name in ['name','description','directories','include_tests']:
        value=metadata[name]
        value=[value] if name=='directories' else value=='true' if name=='include_tests' else value
        suite.replace(module.RegexSqlParsingChecker,name,property(lambda self,v=value:v))
    from scripts.lint_lib.ast_collectors import AstEventCollector
    from scripts.lint_lib.linter_plugin import StructuredViolation
    def factory(root):
        class Collector(AstEventCollector):
            def __init__(self,path,source):self.path=path;self.source=source;self.tree=None;self.calls=[]
            def on_Call(self,node,context):self.calls.append(node)
            def on_node(self,node,context):
                if self.tree is None:
                    while context.parent(node) is not None:node=context.parent(node)
                    self.tree=node
            def finish_file(self):
                tree=self.tree or ast.Module(body=[],type_ignores=[])
                rows=graph(suite,'collector',tree,self.path);nodes=[];pending=[tree]
                while pending:
                    node=pending.pop();nodes.append(node);pending.extend(reversed(list(ast.iter_child_nodes(node))))
                callback_batch.update({id(node):{} for node in self.calls})
                for row in rows:
                    if row['code']=='RSP_FACT':callback_batch[id(nodes[row['parameters']['ordinal']])][row['parameters']['handler']]=violations(module,[row],False)[0]
                checker=module.RegexSqlParsingChecker();result=[]
                try:
                    for node in self.calls:
                        for method in [checker._check_re_call,checker._check_re_compile]:
                            value=method(node,self.path,self.source)
                            if value is not None:result.append(value)
                finally:callback_batch.clear()
                return {'violations':[dict(vars(v)) for v in result]}
        return {module.REGEX_SQL_PARSING_EXTRACTOR.extractor_id:Collector}
    suite.replace(module,'regex_sql_parsing_fact_extractors',factory)
    def reporter(view):
        result=[]
        for path,bundle in view.files.items():
            fact=bundle.facts.get(module.REGEX_SQL_PARSING_EXTRACTOR.extractor_id)
            if not isinstance(fact,dict):continue
            rows=fact.get('violations',[])
            if not isinstance(rows,list):continue
            for row in rows:
                if not isinstance(row,dict):continue
                result.append(StructuredViolation(linter_name='regex_sql_parsing',code=str(row.get('category') or 'REGEX_SQL'),path=path,line=int(row['line']),message=str(row['message']),severity=str(row['severity'])))
        return sorted(result,key=lambda r:r.stable_key)
    suite.replace(module,'_report_regex_sql_parsing',reporter)

    import re
    constants=suite.native('regex_sql_parsing','constants',[])
    for family,name in [('false','_FALSE_POSITIVE_INDICATORS'),('sql','_SQL_SYNTAX_INDICATORS'),('error','_ERROR_MSG_INDICATORS')]:
        data=sorted((r['parameters'] for r in constants if r['parameters']['family']==family),key=lambda p:p['ordinal'])
        suite.replace(module,name,[re.compile(p['pattern'],p['flags']) for p in data])
    count_cache={}
    def result_counts(result):
        # All count properties consume one native aggregate batch. A mutation
        # changes the key; repeated property reads reuse the same complete rows.
        key=tuple((v.category,v.severity) for v in result.violations)
        if key not in count_cache:
            count_cache[key]=suite.native('regex_sql_parsing','result_counts',[],dict(resultCategories=[c for c,_ in key],resultSeverities=[s for _,s in key]))
        return count_cache[key]
    for attribute,key in [('count','total'),('error_count','error'),('warning_count','warning')]:
        def count_property(name):
            def get(result):return next(r['parameters']['value'] for r in result_counts(result) if r['code']=='RSP_COUNT' and r['parameters']['key']==name)
            return property(get)
        suite.replace(module.CheckResult,attribute,count_property(key))
    def category_counts(result):
        rows=sorted((r['parameters'] for r in result_counts(result) if r['code']=='RSP_CATEGORY_COUNT'),key=lambda p:p['first'])
        return {p['key']:p['value'] for p in rows}
    suite.replace(module.CheckResult,'count_by_category',category_counts)
