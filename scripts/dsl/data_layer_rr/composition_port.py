"""Native composition source transport. Application probes have a separate contract."""
import json
CHECK='plan_composition_architecture'


def install(suite,module,detectors=False):
    def scan_semantic_source(path,source):
        rows=suite.native(CHECK,'scan_semantic_source',[dict(path='input.py',source=source)],options=dict(compositionFilename=str(path)))
        errors=[r for r in rows if r['code']=='SPC_SYNTAX']
        if errors:
            fields=errors[0]['parameters']
            def tuples(value):return tuple(tuples(v) for v in value) if isinstance(value,list) else value
            args=tuples(json.loads(fields['exception_args']))
            raise {'SyntaxError':SyntaxError,'IndentationError':IndentationError,'TabError':TabError}[fields['exception_kind']](*args)
        if any(r['code'] not in {'SPC-001','SPC-002','SPC-003','SPC-004','SPC-005'} for r in rows):raise AssertionError('Unexpected composition finding')
        rows.sort(key=lambda r:tuple(r['parameters'][k] for k in ['phase','owner','method','step','depth','ordinal']))
        return tuple(module.CompositionViolation(r['code'],path,r['line'],r['message']) for r in rows)
    suite.replace(module,'scan_semantic_source',scan_semantic_source)
