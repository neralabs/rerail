"""Generate FieldInfo contracts whose expected values come only from the pinned source."""
import argparse
import json
from pathlib import Path


def cases():
    rows = []
    def add(identity, source, path='input.py'):
        rows.append(dict(id=identity, check='unresolved_forward_refs', entry='analyze_file',
            files=[dict(path=path, source=source)], arguments=dict(file_path=dict(path=path))))
    functions = ['get_origin', 'get_args', 'get_type_hints', 'isinstance', 'issubclass', 'TypeAdapter']
    for variable in ['field_info', 'FIELD_INFO', 'some_field_info', 'fk_info', 'f', 'F', 'field', 'fi']:
        for function in functions:
            add(variable + '-' + function, f'def check({variable}):\n    return {function}({variable}.annotation)\n')
    for i, receiver in enumerate(["M.model_fields['x']", "model_fields['x']", "M.other['x']", 'M.model_fields[0]',
                                   "(M.model_fields)['x']", "build().model_fields['x']"]):
        for function in functions:
            add(f'subscript-{i}-{function}', f'def check():\n    return {function}({receiver}.annotation)\n')
    for i, op in enumerate(['is', 'is not', '==', '!=', '<', 'in']):
        for j, other in enumerate(['SomeType', 'some_type', 'None', 'typing.SomeType', '(str,int)', "'X'", 'False']):
            add(f'comparison-{i}-{j}', f'def check(field_info):\n    return field_info.annotation {op} {other}\n')
    for i, detection in enumerate(['str', 'ForwardRef', 'TypeVar', 'type', '(str, type)', '(type,)',
        '(typing.ForwardRef,)', 'typing.ForwardRef', '*[str]', '((str,),)', '(str,)', '(str, *others)']):
        add(f'detection-type-{i}', f'def check(field_info):\n    return isinstance(field_info.annotation, {detection})\n')
    patterns = {
        'module-context': 'get_origin(field_info.annotation)\n',
        'async-context': 'async def check(field_info):\n    return get_origin(field_info.annotation)\n',
        'qualified-call': 'def check(field_info):\n    return typing.get_origin(field_info.annotation)\n',
        'parenthesized-call': 'def check(field_info):\n    return (get_origin)(((field_info.annotation)))\n',
        'keyword-argument': 'get_origin(arg=field_info.annotation)\n',
        'starred-argument': 'get_origin(*(field_info.annotation,))\n',
        'safe-str': 'str(field_info.annotation)\n',
        'qualified-str': 'obj.str(field_info.annotation)\n',
        'reverse-is': 'SomeType is field_info.annotation\n',
        'reverse-equality': 'SomeType == field_info.annotation\n',
        'none-anywhere': 'field_info.annotation is SomeType == None\n',
        'none-left': 'None == field_info.annotation is SomeType\n',
        'first-is-wins': 'field_info.annotation == SomeType is lower\n',
        'first-generic-is-wins': 'SomeType is field_info.annotation is Other\n',
        'first-equality-wins': 'field_info.annotation == SomeType != Other\n',
        'commented-comparison': 'x = (field_info.annotation\n    # comparator\n    is SomeType)\n',
        'attribute-receiver': 'get_origin(outer.field_info.annotation)\n',
        'nested-annotation': 'get_origin(field_info.annotation.annotation)\n',
        'assignment-target': 'field_info.annotation = SomeType\n',
        'attribute-chain-parent': 'field_info.annotation.method()\n',
        'raw-trigger-absent': 'get_origin(field_info.annotat𝚒on)\n',
        'raw-trigger-present': '# .annotation\nget_origin(field_info.annotat𝚒on)\n',
        'nfkc-name': 'get_origin(𝚏.annotation)\n',
        'nfkc-function': 'def 𝚌heck(field_info):\n    return get_origin(field_info.annotation)\n',
        'decorator-context': '@deco(get_origin(field_info.annotation))\ndef check():\n    pass\n',
        'default-context': 'def check(x=get_origin(field_info.annotation)):\n    pass\n',
        'return-annotation-context': 'def check() -> get_origin(field_info.annotation):\n    pass\n',
        'lambda-keeps-context': 'def check():\n    return lambda: get_origin(field_info.annotation)\n',
        'class-keeps-context': 'def check():\n    class Nested:\n        x = get_origin(field_info.annotation)\n',
        'known-fallback': 'resolved_hints.get(name, get_origin(field_info.annotation))\n',
        'custom-unresolved': 'custom.get(name, get_origin(field_info.annotation))\n',
        'resolved-before': 'def check():\n    custom = get_type_hints(Model)\n    return custom.get(name, get_origin(field_info.annotation))\n',
        'resolved-after': 'def check():\n    result = custom.get(name, get_origin(field_info.annotation))\n    custom = get_type_hints(Model)\n',
        'resolved-qualified': 'def check():\n    custom = typing.get_type_hints(Model)\n    return custom.get(name, get_origin(field_info.annotation))\n',
        'nested-assignment-contributes': 'def check():\n    def inner():\n        custom = get_type_hints(Model)\n    return custom.get(name, get_origin(field_info.annotation))\n',
        'outer-does-not-resolve-inner': 'def outer():\n    custom = get_type_hints(Model)\n    def inner():\n        return custom.get(name, get_origin(field_info.annotation))\n',
        'module-assignment-not-discovered': 'custom = get_type_hints(Model)\ncustom.get(name, get_origin(field_info.annotation))\n',
        'annotated-assignment-not-discovered': 'def check():\n    custom: dict = get_type_hints(Model)\n    return custom.get(name, get_origin(field_info.annotation))\n',
        'tuple-assignment-not-discovered': 'def check():\n    custom, other = get_type_hints(Model)\n    return custom.get(name, get_origin(field_info.annotation))\n',
        'walrus-not-discovered': 'def check():\n    (custom := get_type_hints(Model))\n    return custom.get(name, get_origin(field_info.annotation))\n',
        'chained-assignment': 'def check():\n    other = custom = get_type_hints(Model)\n    return custom.get(name, get_origin(field_info.annotation))\n',
        'fallback-first-argument-not-safe': 'resolved_hints.get(get_origin(field_info.annotation), default)\n',
        'fallback-keyword-not-safe': 'resolved_hints.get(name, default=get_origin(field_info.annotation))\n',
        'fallback-deep': 'resolved_hints.get(name, Box(TypeAdapter(field_info.annotation)))\n',
        'different-functions': 'def first():\n    custom = get_type_hints(Model)\ndef second():\n    return custom.get(name, get_origin(field_info.annotation))\n',
        'conditional-field-order': 'get_origin(field_info.annotation) if TypeAdapter(f.annotation) else get_args(fk_info.annotation)\n',
        'two-same-line': 'get_origin(field_info.annotation);get_origin(field_info.annotation)\n',
        'long-snippet': 'get_origin(field_info.annotation) # ' + 'padding' * 180 + '\n',
        'strip-unit-separator': 'get_origin(field_info.annotation) # comment\x1f\n',
        'logical-line-offset': '# header\x1c\nget_origin(field_info.annotation)\n',
        'syntax-error': 'def check(:\n    get_origin(field_info.annotation)\n',
        'clean-text': '# field_info.annotation\nx = "get_origin(field_info.annotation)"\n',
    }
    for identity, text in patterns.items():
        add(identity, text)
    add('non-python-suffix', 'get_origin(field_info.annotation)\n', 'input.txt')
    rows.append(dict(id='missing-file', check='unresolved_forward_refs', entry='analyze_file',
        files=[], arguments=dict(file_path=dict(path='missing.py'))))
    rows.append(dict(id='directory-read', check='unresolved_forward_refs', entry='analyze_file',
        files=[dict(path='directory.py/child.txt', source='')], arguments=dict(file_path=dict(path='directory.py'))))
    rows.append(dict(id='invalid-utf8', check='unresolved_forward_refs', entry='analyze_file',
        files=[dict(path='input.py', bytesHex='ff')], arguments=dict(file_path=dict(path='input.py'))))
    rows.append(dict(id='relative-api-path', check='unresolved_forward_refs', entry='analyze_file', cwd='fixture',
        files=[dict(path='input.py', source='get_origin(field_info.annotation)\n')],
        arguments=dict(file_path=dict(path='input.py', relative=True))))
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--destination', type=Path, required=True)
    parser.add_argument('--reporters', type=Path, required=True)
    args = parser.parse_args()
    rows = cases()
    args.destination.write_text(json.dumps(dict(format='rerail.rr-fresh-contract-cases/v1', cases=rows), indent=2) + '\n')
    reporters = [dict(id=row['id'], check=row['check'], files=[dict(file, path='api/' + file['path']) for file in row['files']])
        for row in rows if not any('bytesHex' in file for file in row['files']) and row['id'] != 'directory-read']
    for path in ['api/a.py', 'impl/a.py', 'api/tests/a.py', 'api/test/a.py', 'api/test_file.py',
                 'api/__pycache__/a.py', 'api/.git/a.py', 'api/venv/a.py', 'api/.venv/a.py', 'api/tmp/a.py', 'public/a.py', 'root.py']:
        reporters.append(dict(id='scope-' + path, check='unresolved_forward_refs', files=[dict(path=path, source='get_origin(field_info.annotation)\n')]))
    args.reporters.write_text(json.dumps(dict(format='rerail.rr-reporter-cases/v1', cases=reporters), indent=2) + '\n')
    print(json.dumps(dict(publicCases=len(rows), reporterCases=len(reporters))))


if __name__ == '__main__':
    main()
