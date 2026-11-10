"""Native composition source transport. Application probes have a separate contract."""
CHECK='plan_composition_architecture'


def install(suite,module,detectors=False):
    def scan_semantic_source(path,source):
        rows=suite.native(CHECK,'scan_semantic_source',[dict(path='input.py',source=source)])
        errors=[r for r in rows if r['code']=='SPC_SYNTAX']
        if errors:
            if errors[0]['message']=='source code string cannot contain null bytes':raise SyntaxError(errors[0]['message'])
            raise NotImplementedError('Exact native SyntaxError location DTO is required: '+errors[0]['message'])
        if any(r['code'] not in {'SPC-001','SPC-002','SPC-003','SPC-004','SPC-005'} for r in rows):raise AssertionError('Unexpected composition finding')
        rows.sort(key=lambda r:tuple(r['parameters'][k] for k in ['phase','owner','method','step','depth','ordinal']))
        return tuple(module.CompositionViolation(r['code'],path,r['line'],r['message']) for r in rows)
    suite.replace(module,'scan_semantic_source',scan_semantic_source)
