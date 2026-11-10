"""Generate the pinned plugin's metadata and fact input namespace."""
from dataclasses import asdict,fields
import json
from pathlib import Path
import tempfile
from data_layer_rr_text import module


def paths_suffix(source):
    original=module(source,'dynamic_module_loading')
    with tempfile.TemporaryDirectory(prefix='dynamic-offline-metadata-') as root:
        plugin=original.dynamic_module_loading_plugin(Path(root))
        metadata={f.name:getattr(plugin,f.name) for f in fields(plugin) if f.name not in {'reporter','file_scope','required_facts'}}
        scope=asdict(plugin.file_scope);scope.pop('directories')
    return '#include "data-layer/dynamic-plugin-metadata.rr"\ndmp_metadata('+json.dumps(json.dumps(dict(plugin=metadata,scope=scope)))+').\n'


def facts_program(source,target,header):
    original=module(source,'dynamic_module_loading');ns=original.DYNAMIC_MODULE_LOADING_EXTRACTOR.extractor_id.replace('~','~0').replace('/','~1')
    text=header('dynamic_module_loading',target).replace('#include "python/ast.rr"','#include "project.rr"')
    text='\n'.join('.scope dirs "__dynamic_no_primary__"' if line.startswith('.scope include ') else line for line in text.splitlines() if not line.startswith('.scope exclude '))+'\n.scope text "**/*"\n'
    text+='#include "data-layer/dynamic-supplied.rr"\n'
    prefix='/bundles/*/facts/'+ns
    for rel,path,projection in [
        ('dmp_container',prefix+'/kind','python-str'),('dmp_shape',prefix+'/rows/*/kind','python-str'),
        ('dmp_member',prefix+'/rows/*/fields/*/key','python-str'),('dmp_value',prefix+'/rows/*/fields/*/encoded','python-value-str'),
        ('dmp_tag',prefix+'/rows/*/fields/*/encoded/0','python-str'),('dmp_scalar',prefix+'/rows/*/fields/*/encoded/1','python-str'),
        ('dmp_json',prefix+'/rows/*/fields/*/encoded/1','json')]:
        text+=f'.input {rel}(IO=json,filename="reporter-facts.json",path={json.dumps(path)},projection={json.dumps(projection)})\n'
    return text
