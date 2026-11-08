"""Record Python decimal conversion offline; compare one native batch without Python execution."""
import argparse
import json
from pathlib import Path
import sys
import unicodedata

from migration import ROOT, sha, write_json


FORMAT = 'rerail.python-integer-contract/v1'


def cases():
    values = ['', ' ', '+', '-', '0', '-0', '+1_2', '1__2', '_1', '1_', '+_1',
              '1.0', '0x10', '1 2', '−1', '²', '\ufeff1', '\x1c1', '1\x1f',
              '9223372036854775807', '-9223372036854775808', '9223372036854775808',
              '-9223372036854775809', '9' * 4301, '-' + '9' * 4301, '0' * 4301]
    for code in range(sys.maxunicode + 1):
        char = chr(code)
        if unicodedata.category(char) == 'Nd' and unicodedata.decimal(char) == 0:
            values.extend([chr(code + 1) + '_' + chr(code + 2), '-' + chr(code + 9)])
        if char.isspace():
            values.append(char + '12' + char)
    result = []
    for text in values:
        for low, high in [(1, 52), (-(2**63), 2**63 - 1)]:
            result.append(dict(text=text, lower=low, upper=high, maxDigits=4300))
    for limit in [640, 4300, 0]:
        for length in [639, 640, 641, 4299, 4300, 4301]:
            result.append(dict(text='0' * length, lower=1, upper=52, maxDigits=limit))
    return result


def source(records):
    previous = sys.get_int_max_str_digits()
    result = []
    try:
        for row in records:
            sys.set_int_max_str_digits(row['maxDigits'])
            try:
                value = max(row['lower'], min(row['upper'], int(row['text'])))
            except ValueError:
                value = None
            result.append(dict(input=row, expected=value))
    finally:
        sys.set_int_max_str_digits(previous)
    return result


def program(records):
    text = '''.policy "python-integer-contract" revision 1
.language python
.mode log
.scope dirs "__no_primary__"
.decl inputs(row: number, text: symbol, lower: number, upper: number, digits: number)
'''
    for i, row in enumerate(records):
        p = row['input']
        text += f'inputs({i}, {json.dumps(p["text"],ensure_ascii=False)}, {p["lower"]}, {p["upper"]}, {p["maxDigits"]}).\n'
    return text + '''
.violation Parsed(at: span, row: number, value: number) code "PARSED_INTEGER" "parsed"
Parsed(@place("",0), i, value) :- inputs(i,text,lo,hi,digits), value = @python_int_clamped(text,lo,hi,digits), value >= lo.
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['source', 'native'])
    for name in ['engine', 'baseline', 'output']:
        parser.add_argument('--' + name, type=Path, required=name == 'output')
    args = parser.parse_args()
    result = dict(format=FORMAT, complete=False, passed=False)
    if args.command == 'source':
        if sys.version_info[:2] != (3, 13) or unicodedata.unidata_version != '15.1.0':
            parser.error('The recorded oracle requires Python 3.13 and Unicode 15.1')
        result.update(python=sys.version, unicode=unicodedata.unidata_version, cases=source(cases()))
    else:
        from replay import execute_native
        baseline = json.loads(args.baseline.read_text())
        if baseline['format'] != FORMAT or not baseline['complete'] or not baseline['passed']:
            parser.error('Complete source recording required')
        if [r['input'] for r in baseline['cases']] != cases():
            parser.error('Complete unchanged input matrix required')
        path = ROOT / '.rerail/rr-migration-20261106/internal-dead-code/weighted/integers.rr'
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(program(baseline['cases']))
        native = execute_native(args.engine, path, [], {})
        actual = {}
        for row in native['diagnostics']:
            if row['code'] != 'PARSED_INTEGER':
                raise ValueError('Unexpected native diagnostic')
            p = row['parameters']
            if p['row'] in actual or not 0 <= p['row'] < len(baseline['cases']):
                raise ValueError('Repeated or unknown input occurrence')
            actual[p['row']] = p['value']
        result.update(cases=[dict(row, actual=actual.get(i), passed=actual.get(i) == row['expected'])
                             for i, row in enumerate(baseline['cases'])], native=native,
                      engineSha256=sha(args.engine.read_bytes()), baselineSha256=sha(args.baseline.read_bytes()))
    result.update(complete=True, passed=all(c.get('passed', True) for c in result['cases']))
    write_json(args.output, result)
    print(json.dumps(dict(cases=len(result['cases']), passed=result['passed'])))
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
