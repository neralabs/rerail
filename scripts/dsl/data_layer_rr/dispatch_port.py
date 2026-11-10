"""Transport dispatch source and passive findings; incomplete provenance fails explicitly."""
CHECK='serverplan_dispatch_architecture'


def install(backend,module,detectors=False):
    inventories={}
    def scan_source(path,source):
        rows=backend.native(CHECK,'scan_source',[dict(path='input.py',source=source)],dict(dispatchPath=str(path)))
        if any(r['code']=='DISPATCH_INCOMPLETE' for r in rows):raise NotImplementedError('BaseServerPlan executor provenance is not ported')
        if any(r['code'] not in {'SPD001','SPD002','SPD003','SPD004'} for r in rows):raise AssertionError('Unexpected dispatch output')
        rows.sort(key=lambda r:(r['line'],r['code'],r['parameters']['expression'],r['parameters']['occurrence']))
        result=[]
        for r in rows:
            finding=module.Finding(r['code'],path,r['line'],r['parameters']['scope'],r['parameters']['expression'],r['message'],r['parameters']['occurrence'])
            inventories[id(finding)]=(finding,r['parameters']['inventory']);result.append(finding)
        return tuple(result)
    def inventory(finding):
        row=inventories.get(id(finding))
        if row is None or row[0] is not finding:raise NotImplementedError('Caller-created dispatch finding projection is not ported')
        return row[1]
    def inventory_is_empty(items):return not backend.native(CHECK,'inventory_is_empty',[],dict(dispatchInventory=list(items)))
    def unapproved_findings(findings,approved_entries):
        rows=backend.native(CHECK,'unapproved_findings',[],dict(dispatchInventory=[inventory(f) for f in findings],dispatchApproved=sorted(approved_entries)))
        if any(r['code']!='DISPATCH_UNAPPROVED' for r in rows):raise AssertionError('Unexpected dispatch inventory response')
        return tuple(findings[r['parameters']['ordinal']] for r in sorted(rows,key=lambda r:r['parameters']['ordinal']))
    backend.replace(module,'scan_source',scan_source)
    backend.replace(module.Finding,'inventory_entry',property(inventory))
    backend.replace(module,'inventory_is_empty',inventory_is_empty)
    backend.replace(module,'unapproved_findings',unapproved_findings)
