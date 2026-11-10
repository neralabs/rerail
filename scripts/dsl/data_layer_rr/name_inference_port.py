"""Transport whole file inventories; native rules select, scan, and format findings."""
import json
from pathlib import Path

CHECK='serverplan_name_inference'


def default_paths(suite,module):
    paths=[p for root in module._SCAN_ROOTS for p in root.rglob('*.py')]
    rows=suite.native(CHECK,'_name_paths',[],dict(namePaths=[str(p) for p in paths]))
    if any(r['code']!='NINF_PATH' for r in rows):raise AssertionError('Unexpected default-path response')
    return tuple(Path(r['parameters']['value']) for r in sorted(rows,key=lambda r:r['parameters']['ordinal']))


def scan(suite,entry,module,arguments):
    if entry in {'_scan_source','_extract_name_inference_facts'}:
        paths=[Path(arguments['path'])];files=[dict(path='input-0.py',source=arguments['source'])]
    else:
        supplied=arguments.get('paths')
        # Truthiness belongs to the public iterable protocol. An empty generator
        # is truthy, while an empty tuple/list requests the default paths.
        paths=list(supplied or (module._default_paths() if hasattr(module,'_default_paths') else default_paths(suite,module)))
        files=[dict(path=f'input-{i}.py',source=p.read_text()) for i,p in enumerate(paths)]
    rows=suite.native(CHECK,entry,files,dict(nameDocs=[f['path'] for f in files],nameBasenames=[p.name for p in paths]))
    if entry=='violations':
        if any(r['code'] not in {'SPNI001','SPNI002','SPNI003','SPNI004','NINF_TEXT'} for r in rows):raise AssertionError('Unexpected name-inference finding')
        return [r['parameters']['value'] for r in sorted((r for r in rows if r['code']=='NINF_TEXT'),key=lambda r:r['parameters']['value'])]
    if entry=='_extract_name_inference_facts':
        return [json.loads(r['parameters']['value_json']) for r in sorted((r for r in rows if r['code']=='NINF_FACT'),key=lambda r:(r['line'],r['parameters']['ordinal']))]
    if any(r['code'] not in {'SPNI001','SPNI002','SPNI003','SPNI004'} for r in rows):raise AssertionError('Unexpected source finding')
    return [(r['line'],r['code'],r['message']) for r in sorted(rows,key=lambda r:(r['line'],r['code']))]


def install(suite,module,detectors=False):
    rows=suite.native(CHECK,'_name_configuration',[])
    if not rows or any(r['code']!='NINF_CONFIG' for r in rows):raise AssertionError('Missing name-inference configuration')
    config={}
    for r in sorted(rows,key=lambda r:r['parameters']['ordinal']):
        p=r['parameters'];config.setdefault(p['key'],[]).append(p['value'])
    for name in ['_CONTROL_COLUMNS','_REFERENCE_SUFFIXES','_REFERENCE_TEXT_METHODS']:
        suite.replace(module,name,tuple(config[name]))
    suite.replace(module,'_SCAN_DIRECTORIES',tuple(Path(p) for p in config['_SCAN_DIRECTORIES']))
    import re
    for name in ['_COLUMN_SHAPE_RE','_CONTROL_LITERAL_RE','_REFERENCE_PARSE_RE','_REFERENCE_SYNTHESIS_RE','_COLUMN_TYPE_RE']:
        suite.replace(module,name,re.compile(config[name+'.pattern'][0],int(config[name+'.flags'][0])))
    data=json.loads(config['EXTRACTOR'][0])
    for key in ['source_triggers','source_trigger_groups','project_input_facts']:data[key]=tuple(data[key])
    suite.replace(module,'NAME_INFERENCE_EXTRACTOR',type(module.NAME_INFERENCE_EXTRACTOR)(**data))
    suite.replace(module,'_default_paths',lambda:default_paths(suite,module))
    suite.replace(module,'_extract_name_inference_facts',lambda path,source:scan(suite,'_extract_name_inference_facts',module,dict(path=path,source=source)))
    if hasattr(suite,'configuration_bindings'):suite.configuration_bindings[CHECK]=dict(nativeCall=len(suite.calls)-1,names=list(config))
    if detectors:
        suite.replace(module,'_scan_source',lambda path,source:scan(suite,'_scan_source',module,dict(path=path,source=source)))
        suite.replace(module,'violations',lambda paths=None:scan(suite,'violations',module,dict(paths=paths)))
    def cli_main():
        import argparse
        import sys
        parser=argparse.ArgumentParser(description=config['description'][0])
        parser.add_argument('--check',action='store_true',help=config['check_help'][0]);args=parser.parse_args()
        rows=suite.native(CHECK,'_name_cli',[],dict(nameMessages=list(module.violations()),checkMode=str(int(args.check))))
        if any(r['code'] not in {'NINF_OUTPUT','NINF_EXIT'} for r in rows):raise AssertionError('Unexpected CLI response')
        for r in sorted((r for r in rows if r['code']=='NINF_OUTPUT'),key=lambda r:r['parameters']['ordinal']):sys.stdout.write(r['parameters']['stdout'])
        statuses=[r['parameters']['value'] for r in rows if r['code']=='NINF_EXIT']
        if len(statuses)!=1:raise AssertionError('One CLI status required')
        return statuses[0]
    suite.replace(module,'cli_main',cli_main)
