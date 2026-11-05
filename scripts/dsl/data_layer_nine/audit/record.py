# SPDX-License-Identifier: Apache-2.0
"""Record source-shape cases with the original implementation and pinned source verification."""
import argparse,ast,hashlib,importlib,json,sys,tempfile
from dataclasses import asdict
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[2]))
from data_layer_nine.audit.shapes import cases
from data_layer_probe_record import verify_sources


def observe(root,record):
    module=importlib.import_module('scripts.lint_'+record['check']+('_plugin' if record['check']=='schema_architecture' else ''))
    with tempfile.TemporaryDirectory() as directory:
        temporary=Path(directory)
        for file in record['files']:
            path=temporary/file['path'];path.parent.mkdir(parents=True,exist_ok=True);path.write_text(file['source'])
        first=record['files'][0];path=temporary/first['path'];source=first['source']
        if record['check']=='connection_structural_boundaries':
            changes=dict(ROOT=temporary,CONNECTION_FILE=path,CONNECTION_LIFECYCLE_FILE=temporary/'l2000_connection/connection_lifecycle.py',SQL_CONNECTION_FILE=temporary/'missing.py',BACKEND_CONNECTION_FILES=(),LIFECYCLE_SCAN_FILES=())
            saved={name:getattr(module,name) for name in changes}
            try:
                for name,value in changes.items():setattr(module,name,value)
                result=module.lint()
            finally:
                for name,value in saved.items():setattr(module,name,value)
        elif record['entry']=='lint_file':result=module.lint_file(path)
        elif record['entry']=='_collect_fake_executor_violations':result=module._collect_fake_executor_violations(path,ast.parse(source).body[0])
        else:result=getattr(module,record['entry'])(path,source)
        return [{**asdict(row),'path':row.path.relative_to(temporary).as_posix()} for row in result]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,required=True);parser.add_argument('--repo',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();root=args.source.resolve();sys.path[:0]=[str(root),str(root/'scripts')]
    records=[{**record,'expected':observe(root,record)} for record in cases()]
    checked=verify_sources(root,args.repo.resolve(),'2b1504adc4291daaf080b2e932962b134da1a69a')
    args.output.parent.mkdir(parents=True,exist_ok=True)
    from data_layer_nine.evidence import compact
    report=compact(dict(format='rerail.data-layer-source-shapes/v1',sourcePin='2b1504adc4291daaf080b2e932962b134da1a69a',sourceFilesChecked=checked,cases={},sources={},records=records))
    report['sourceSha256']={str(path.relative_to(root)):hashlib.sha256(path.read_bytes()).hexdigest() for module in tuple(sys.modules.values()) if (value:=getattr(module,'__file__',None)) and (path:=Path(value).resolve()).suffix=='.py' and path.is_relative_to(root)}
    report['sourceSha256']=dict(sorted(report['sourceSha256'].items()))
    args.output.write_text(json.dumps(report,indent=2)+'\n')


if __name__=='__main__':main()
