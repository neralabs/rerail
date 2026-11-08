"""Transport grammar escape-hatch arguments and DTOs; rule decisions belong to native relations."""
import ast
import sys
from pathlib import Path
from ast_transport import visit_columns

GROUPS = ['ALLOWLISTED_CLASSES','DATA_VALUE_PARAMS','OPERATOR_PARAMS','_SAFE_PARAM_TYPES_STATIC',
          'SAFE_PARAM_TYPES','TRANSPARENT_CONTAINER_TYPES','SCHEMA_METADATA_CLASSES',
          'CALLSITE_PROTECTED_CLASSES','FORBIDDEN_ESCAPE_HATCHES']


def install(suite,module):
    rows=suite.native('sql_grammar_escape_hatches','configuration',[])
    for group in GROUPS:
        suite.replace(module,group,frozenset(r['parameters']['value'] for r in rows
                      if r['code']=='SGE_CONSTANT' and r['parameters']['family']==group))
    for row in rows:
        if row['code']=='SGE_METADATA':
            key,value=row['parameters']['key'],row['parameters']['value']
            suite.replace(module,key,int(value) if key=='MAX_ESCAPE_HATCH_VIOLATIONS' else value)
    default_grammar=frozenset(r['parameters']['value'] for r in rows
        if r['code']=='SGE_CONSTANT' and r['parameters']['family']=='DEFAULT_GRAMMAR_CLASS_NAMES')
    def initialize(visitor,file_path,*,grammar_class_names=None,safe_param_types=None):
        visitor.file_path=file_path
        visitor.grammar_class_names=default_grammar if grammar_class_names is None else grammar_class_names
        visitor.safe_param_types=module.SAFE_PARAM_TYPES if safe_param_types is None else safe_param_types
        visitor.violations=[]
    suite.replace(module.GrammarClassVisitor,'__init__',initialize)
    def argument(entry,tree):
        options=visit_columns(tree);nodes=[];pending=[tree]
        while pending:
            node=pending.pop();nodes.append(node);pending.extend(reversed(list(ast.iter_child_nodes(node))))
        options['astConstantText']=[str(n.value) if isinstance(n,ast.Constant) else '' for n in nodes]
        return suite.native('sql_grammar_escape_hatches',entry,[],options)
    def types(entry,tree):
        return {r['parameters']['name'] for r in argument(entry,tree) if r['code']=='SGE_TYPE'}
    def leaf(visitor,node):return types('GrammarClassVisitor._extract_leaf_types',node)
    suite.replace(module.GrammarClassVisitor,'_extract_leaf_types',leaf)
    suite.replace(module,'_extract_alias_targets',lambda node:types('_extract_alias_targets',node))
    for method in ['_get_name','_annotation_to_string']:
        def factory(name):
            def evaluate(visitor,node):
                return next((r['parameters']['value'] for r in argument('GrammarClassVisitor.'+name,node) if r['code']=='SGE_VALUE'),None)
            return evaluate
        suite.replace(module.GrammarClassVisitor,method,factory(method))
    def crypto(entry):
        return suite.native('sql_grammar_escape_hatches',entry,[],dict(allowlist=list(module.ALLOWLISTED_CLASSES),
            publicKey=module.ALLOWLIST_PUBLIC_KEY_HEX,signature=module.ALLOWLIST_SIGNATURE))
    def message():
        return next(r['parameters']['value'] for r in crypto('_compute_signature_message') if r['code']=='SGE_VALUE').encode('utf-8')
    def public_key():
        return bytes.fromhex(next(r['parameters']['value'] for r in crypto('_get_public_key_bytes') if r['code']=='SGE_VALUE'))
    suite.replace(module,'_compute_signature_message',message)
    suite.replace(module,'_get_public_key_bytes',public_key)
    suite.replace(module,'_verify_allowlist_integrity',lambda:any(r['code']=='SGE_TRUE' for r in crypto('_verify_allowlist_integrity')))
    def usages_source(file_path,source):
        rows=suite.native('sql_grammar_escape_hatches','scan_usages',[
            dict(path='impl/__supplied__.py',source=source)],dict(originalPath=str(file_path)))
        rows=sorted((r for r in rows if r['code']=='SGE_USAGE'),key=lambda r:(r['line'],r['parameters']['ordinal']))
        return [module.EscapeHatchViolation(file_path=r['path'],line_number=r['line'],
                    class_name=r['parameters']['class_name'],reason=r['message']) for r in rows]
    def usages(file_path):
        try:source=file_path.read_text()
        except (UnicodeDecodeError,OSError):return []
        return usages_source(file_path,source)
    suite.replace(module,'scan_usages',usages)
    suite.replace(module,'scan_usages_source',usages_source)
    def report(violations,check_mode):
        rows=suite.native('sql_grammar_escape_hatches','_report_violations',[],dict(
            reportPaths=[v.file_path for v in violations],reportLines=[str(v.line_number) for v in violations],
            reportClasses=[v.class_name for v in violations],reportReasons=[v.reason for v in violations],
            ceiling=module.MAX_ESCAPE_HATCH_VIOLATIONS,checkMode=check_mode))
        row=next(r for r in rows if r['code']=='SGE_REPORT')['parameters']
        print(row['output'],end='')
        return row['status']
    suite.replace(module,'_report_violations',report)
    def class_files(entry,paths,directories=(),configuration=None):
        files=[];original=[];logical=[]
        for i,path in enumerate(dict.fromkeys(paths)):
            name='impl/__supplied_'+str(i)+'.py'
            try:source=path.read_text()
            except UnicodeDecodeError as error:
                if entry!='scan_file':raise
                print(f'Warning: Could not parse {path}: {error}',file=sys.stderr)
                return []
            files.append(dict(path=name,source=source))
            original.append(str(path));logical.append(name)
        options=dict(originalPaths=original,logicalPaths=logical,directoryPaths=[str(p) for p in directories],originalNames=[Path(p).name for p in original])
        options.update(configuration or {})
        rows=suite.native('sql_grammar_escape_hatches',entry,files,options)
        for row in rows:
            if row['code']=='SGE_WARNING':print(row['message'],file=sys.stderr)
        rows=sorted((r for r in rows if r['code']=='SGE_FILE'),key=lambda r:(r['path'],r['parameters']['phase'],r['parameters']['ordinal']))
        return [module.EscapeHatchViolation(file_path=r['path'],line_number=r['line'],class_name=r['parameters']['class_name'],reason=r['message']) for r in rows]
    def scan_file(file_path,*,grammar_class_names=None,safe_param_types=None):
        return class_files('scan_file',[file_path],configuration=dict(
            customGrammarProvided=grammar_class_names is not None,customSafeProvided=safe_param_types is not None,
            grammarClassNames=list(grammar_class_names or ()),safeParamTypes=list(safe_param_types or ())))
    suite.replace(module,'scan_file',scan_file)
    def collect(paths):
        files=[];directories=[]
        for value in paths:
            path=Path(value)
            if path.is_file() and path.suffix=='.py':files.append(path)
            elif path.is_dir():
                discovered=list(path.rglob('*.py'));files.extend(discovered);directories.extend(discovered)
        if not files:return []
        return class_files('_collect_violations',files,directories)
    suite.replace(module,'_collect_violations',collect)
