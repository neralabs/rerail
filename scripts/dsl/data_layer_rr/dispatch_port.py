"""Transport dispatch source and passive findings; incomplete provenance fails explicitly."""
CHECK='serverplan_dispatch_architecture'


def install(backend,module,detectors=False):
    def scan_source(path,source):
        rows=backend.native(CHECK,'scan_source',[dict(path='input.py',source=source)],dict(dispatchPath=str(path)))
        if any(r['code']=='DISPATCH_INCOMPLETE' for r in rows):raise NotImplementedError('BaseServerPlan executor provenance is not ported')
        if any(r['code'] not in {'SPD001','SPD002','SPD003','SPD004'} for r in rows):raise AssertionError('Unexpected dispatch output')
        rows.sort(key=lambda r:(r['line'],r['code'],r['parameters']['expression'],r['parameters']['occurrence']))
        return tuple(module.Finding(r['code'],path,r['line'],r['parameters']['scope'],r['parameters']['expression'],r['message'],r['parameters']['occurrence']) for r in rows)
    backend.replace(module,'scan_source',scan_source)
