"""Re-bless the production contract fixtures from the engine's actual results.

  RERAIL_CONTRACT_ACTUAL=/path/actual.json <engine test binary> production_contract_actual_results --ignored --exact
  python3 tools/contracts/bless_production_contracts.py /path/actual.json

Writes expected.oracle into fixtures/code-rule-dsl/query/production_contracts_*.json and terminal_catalog into
implementation_goldens_phase_*.json, keeping each file's layout. Review the diff before committing.
"""
import glob, json, os, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FIELDS = ['ordinal', 'kind', 'dialect', 'producer_kind', 'program_digest', 'result_schema', 'owning_clause_ids',
          'result_limit', 'completion_required']
QUERY = os.path.join(ROOT, 'fixtures/code-rule-dsl/query')


def contracts(actual):
    for path in sorted(glob.glob(os.path.join(QUERY, 'production_contracts_*.json'))):
        cases = json.load(open(path))
        for case in cases:
            case['expected']['oracle'] = actual['oracle'][case['name']]
        open(path, 'w').write(json.dumps(cases, indent=2, ensure_ascii=False) + '\n')


def goldens(actual):
    """Each file keeps its own layout: indented (2-4), one line per entry (5, 7), or one line per case (6)."""
    for path in sorted(glob.glob(os.path.join(QUERY, 'implementation_goldens_phase_*.json'))):
        cases = json.load(open(path))['cases']
        for case in cases:
            case['terminal_catalog'] = [{k: e[k] for k in FIELDS} for e in actual['catalogs'][case['name']]]
        if path.endswith('_2_4.json'):
            text = json.dumps({'cases': cases}, indent=2, ensure_ascii=False) + '\n'
        elif path.endswith(('_5.json', '_7.json')):
            seps = (', ', ': ') if path.endswith('_5.json') else (',', ':')
            out = []
            for case in cases:
                body = ',\n'.join('        ' + json.dumps(e, ensure_ascii=False, separators=seps) for e in case['terminal_catalog'])
                out.append('    {\n      "name": ' + json.dumps(case['name']) + ',\n      "terminal_catalog": [\n' + body
                           + '\n      ]\n    }')
            text = '{\n  "cases": [\n' + ',\n'.join(out) + '\n  ]\n}\n'
        else:
            lines = ['    ' + json.dumps(case, ensure_ascii=False, separators=(',', ':')) for case in cases]
            text = '{\n  "cases": [\n' + ',\n'.join(lines) + '\n  ]\n}\n'
        open(path, 'w').write(text)


actual = json.load(open(sys.argv[1]))
contracts(actual)
goldens(actual)
