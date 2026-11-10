"""Transport dispatch source and passive findings; migration admission remains pending."""
from pathlib import Path

CHECK='serverplan_dispatch_architecture'


def install(backend,module,detectors=False):
    inventories={}
    rendered={}
    def key(finding):
        return tuple(str(getattr(finding,n)) for n in ['code','path','line','scope','expression','message','occurrence'])
    def project(findings,roots):
        findings=tuple(findings);roots=tuple(Path(r) for r in roots)
        if len(findings)!=len(roots):raise ValueError('One root is required per Finding')
        options={'dispatchFinding'+n.title():[str(getattr(f,n)) for f in findings]
                 for n in ['code','path','line','scope','expression','message','occurrence']}
        options['dispatchFindingRoot']=[str(r) for r in roots]
        rows=backend.native(CHECK,'_finding_projection',[],options)
        if any(r['code']!='SD_FINDING' for r in rows):raise AssertionError('Unexpected Finding projection')
        values=sorted((r['parameters'] for r in rows),key=lambda r:r['ordinal'])
        if [r['ordinal'] for r in values]!=list(range(len(findings))):raise AssertionError('Missing or duplicate Finding projection')
        for f,root,row in zip(findings,roots,values):
            inventories[key(f)]=row['inventory'];rendered[(key(f),str(root))]=row['rendered']
        return values
    def scan_source(path,source):
        rows=backend.native(CHECK,'scan_source',[dict(path='input.py',source=source)],dict(dispatchPath=str(path)))
        if any(r['code'] not in {'SPD001','SPD002','SPD003','SPD004','SPD005'} for r in rows):raise AssertionError('Unexpected dispatch output')
        rows.sort(key=lambda r:(r['line'],r['code'],r['parameters']['expression'],r['parameters']['occurrence']))
        result=[]
        for r in rows:
            finding=module.Finding(r['code'],path,r['line'],r['parameters']['scope'],r['parameters']['expression'],r['message'],r['parameters']['occurrence'])
            inventories[key(finding)]=r['parameters']['inventory'];result.append(finding)
        return tuple(result)
    def inventory(finding):
        identity=key(finding)
        if identity not in inventories:project([finding],[module.ROOT])
        return inventories[identity]
    default_root=module.Finding.render.__defaults__[0]
    def render(finding,root=default_root):
        identity=(key(finding),str(Path(root)))
        if identity not in rendered:project([finding],[root])
        return rendered[identity]
    def inventory_is_empty(items):return not backend.native(CHECK,'inventory_is_empty',[],dict(dispatchInventory=list(items)))
    def unapproved_findings(findings,approved_entries):
        rows=backend.native(CHECK,'unapproved_findings',[],dict(dispatchInventory=[inventory(f) for f in findings],dispatchApproved=sorted(approved_entries)))
        if any(r['code']!='DISPATCH_UNAPPROVED' for r in rows):raise AssertionError('Unexpected dispatch inventory response')
        return tuple(findings[r['parameters']['ordinal']] for r in sorted(rows,key=lambda r:r['parameters']['ordinal']))
    backend.replace(module,'scan_source',scan_source)
    backend.replace(module.Finding,'inventory_entry',property(inventory))
    backend.replace(module.Finding,'render',render)
    backend.replace(module,'inventory_is_empty',inventory_is_empty)
    backend.replace(module,'unapproved_findings',unapproved_findings)
    return project
