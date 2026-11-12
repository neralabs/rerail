"""Transport shared ratchet I/O and signing; execute baseline decisions in native SQL."""
from datetime import UTC
from pathlib import Path
import subprocess
import sys


class NativeRatchet:
    def __init__(self,backend,check,namespace):self.backend=backend;self.check=check;self.namespace=namespace
    def query(self,entry,files=(),options=None):return self.backend.native(self.check,entry,list(files),options or {})
    def result(self,rows):
        for p in sorted((r['parameters'] for r in rows if r['code']=='IR_OUTPUT'),key=lambda p:p['ordinal']):sys.stdout.write(p['stdout']);sys.stderr.write(p['stderr'])
        errors=[r['parameters'] for r in rows if r['code']=='IR_ERROR']
        if errors:
            p=errors[0];raise {'AttributeError':AttributeError,'NotImplementedError':NotImplementedError,'ValueError':ValueError,'TypeError':TypeError,'OverflowError':OverflowError}[p['error_type']](p['detail'])
        values=[r['parameters']['value'] for r in rows if r['code']=='IR_RESULT']
        if len(values)!=1:raise ValueError('One native ratchet result required')
        return bool(values[0])
    def check_baseline(self,name,counts,*,root=None,require_signature=True,require_baseline=False):
        root=Path(root) if root is not None else self.namespace.get_repo_root();path=root/'.ratchet-baselines'/(name+'.json');files=[]
        options=dict(ratchetName=name,ratchetPath=str(path),requireSignature=str(int(require_signature)),requireBaseline=str(int(require_baseline)),currentKeys=list(counts),currentValues=[str(v) for v in counts.values()],missingBaseline=str(int(not path.exists())))
        if path.exists():
            try:text=path.read_text();files=[dict(path='baseline.json',source=text)];options['baselineText']=text
            except Exception as error:options['readError']=str(error)
        return self.result(self.query('_ratchet_check',files,options))
    def update(self,name,counts,*,root=None,sign=False,reason=''):
        root=Path(root) if root is not None else self.namespace.get_repo_root();path=root/'.ratchet-baselines'/(name+'.json');path.parent.mkdir(parents=True,exist_ok=True)
        options=dict(ratchetName=name,ratchetPath=str(path),currentKeys=list(counts),currentValues=[str(v) for v in counts.values()]);signature='UNSIGNED'
        if sign:
            requests=self.query('_ratchet_sign_request',options=options)
            if len(requests)!=1 or requests[0]['code']!='IR_SIGN_REQUEST':raise ValueError('One native ratchet signing request required')
            p=requests[0]['parameters'];replies=self.backend.sign_ed25519([dict(id=p['request_id'],key_env=p['key_env'],message=p['message'])])
            if len(replies)!=1 or replies[0]['id']!=p['request_id']:raise ValueError('One matching ratchet signing reply required')
            reply=replies[0]
            if reply['status']!='signed':return self.result(self.query('_ratchet_sign_error',options=dict(options,signStatus=reply['status'],signDetail=reply['detail'])))
            signature=reply['signature']
        approver='unknown'
        try:
            result=subprocess.run(['git','config','user.email'],capture_output=True,text=True,check=False)
            if result.returncode==0 and result.stdout.strip():approver=result.stdout.strip()
        except Exception:pass
        rows=self.query('_ratchet_update',options=dict(options,newSignature=signature,updatedAt=self.namespace.datetime.now(UTC).isoformat(),reason=reason,approver=approver))
        writes=[r['parameters']['contents'] for r in rows if r['code']=='IR_WRITE']
        if len(writes)!=1:raise ValueError('One native ratchet write required')
        path.write_text(writes[0]);return self.result(rows)
    def install(self):
        source=self.namespace.get_repo_root.__globals__['__file__']
        def root():
            rows=self.query('_ratchet_root',options=dict(ratchetDiscoverySource=str(Path(source).resolve())))
            if len(rows)!=1 or rows[0]['code']!='IR_ROOT':raise ValueError('One native ratchet root required')
            return Path(rows[0]['parameters']['path'])
        self.backend.replace(self.namespace,'get_repo_root',root)
        self.backend.replace(self.namespace,'check_ratchet',self.check_baseline)
        self.backend.replace(self.namespace,'update_ratchet_baseline',self.update)
