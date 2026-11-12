"""Re-record the full existing adversarial unresolved-reference public corpus."""
import contextlib
from pathlib import Path
import sys
from contract_runner import main
from fresh import arguments,invoke
from migration import ROOT
from unresolved_components_contract import CHECK


def cases(_module):
    sys.path.insert(0,str(ROOT/'scripts/generate/packs'))
    from data_layer_rr_unresolved_cases import cases as generated
    return [dict(kind='public',**row) for row in generated()]


def exercise(module,case,scratch):
    root=scratch.resolve()/'repo';root.mkdir()
    for file in case['files']:
        path=root/file['path'];path.parent.mkdir(parents=True,exist_ok=True)
        if 'bytesHex' in file:path.write_bytes(bytes.fromhex(file['bytesHex']))
        else:path.write_text(file['source'])
    values=arguments(case['arguments'],root)
    with contextlib.chdir(root if case.get('cwd')=='fixture' else Path.cwd()):
        if case['entry']=='analyze_file':call=lambda:module.analyze_file(**values)
        else:
            from unresolved_port import analyzed_state
            owner=module.AnnotationUsageFinder(file_path=values['file_path'],source_lines=values.get('source_lines',[]),findings=values.get('initial_findings',[]))
            def call():return analyzed_state(owner,owner.analyze(values['tree']))
        return invoke(call,scratch.resolve())


def install(backend,module,detectors=False):
    from unresolved_port import install as install_native
    install_native(backend,module,detectors=detectors)


if __name__=='__main__':main(CHECK,'rerail.unresolved-public/v1',cases,exercise,install,['unresolved_public_contract.py','unresolved_components_contract.py','fresh.py','../../generate/packs/data_layer_rr_unresolved_cases.py'])
