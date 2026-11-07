"""Cold SQL ownership and typed publication over frozen production index facts."""
import argparse
import hashlib
import json
from pathlib import Path
import sqlite3
import tempfile
import time

TABLES = ['lang', 'kind', 'field', 'file', 'fk', 'file_kind_counts', 'kfreq',
          'parovf', 'sizeovf', 'vidx', 'vcov']
CATALOG = ['lang', 'kind', 'field']


class Meter:
    def __init__(self, path, trace=False):
        self.path = Path(path)
        self.db = sqlite3.connect(path, isolation_level=None)
        self.statements = 0
        self.by_phase = {}
        self.phase = 'setup'
        self.traced = trace
        self.trace_executions = 0
        if trace:
            self.db.set_trace_callback(self.observe)

    def observe(self, _sql):
        self.trace_executions += 1

    def execute(self, sql, values=()):
        self.statements += 1
        self.by_phase[self.phase] = self.by_phase.get(self.phase, 0) + 1
        return self.db.execute(sql, values)

    def close(self):
        self.db.close()
        self.path.unlink(missing_ok=True)


def quote(value):
    return '"' + value.replace('"', '""') + '"'


def inputs(raw):
    captures = []
    startup = None
    for path in sorted(raw.glob('candidate-*.json')):
        data = json.loads(path.read_text())
        db = sqlite3.connect(raw / data['database'])
        tables = {}
        for table in TABLES:
            cursor = db.execute('SELECT * FROM ' + quote(table))
            tables[table] = ([column[0] for column in cursor.description], cursor.fetchall())
        counts = dict(db.execute('SELECT fid,nodes FROM file'))
        postings = {fid: [] for fid in counts}
        for fid, term, _offset in db.execute('SELECT doc,term,offset FROM ftsk_inst ORDER BY doc,offset'):
            postings[fid].append(term)
        assert all(len(postings[fid]) == count for fid, count in counts.items())
        tokens = {fid: ' '.join(terms) for fid, terms in postings.items()}
        captures.append((tables, tokens))
        db.close()
    for path in sorted(raw.glob('profile-*.json')):
        catalog = json.loads(path.read_text())
        queries = {query['id']: query['sql'] for query in catalog['queries']}
        events = [json.loads(line) for line in path.with_suffix('.jsonl').read_text().splitlines()]
        sql = [queries[event['query']] for event in events
               if event.get('kind') == 'sql' and event.get('origin') == 'engine'
               and (event.get('context') or {}).get('component') == 'ast-index'
               and (event.get('context') or {}).get('stage') == 'schema']
        if any('CREATE TABLE file (' in query for query in sql):
            startup = sql
            break
    assert startup and len(startup) == 42, len(startup or [])
    assert all(all(tables[name] == captures[0][0][name] for name in CATALOG)
               for tables, _ in captures), 'This component seed requires one closed grammar catalog'
    return captures, startup


def scale(captures, copies):
    """Replicate real tapes at different paths to stress publication and retirement."""
    if copies == 1:
        return captures
    result = []
    for tables, tokens in captures:
        stride = max(tokens) + 1
        expanded = {}
        for table, (columns, rows) in tables.items():
            if table in CATALOG:
                expanded[table] = (columns, rows)
                continue
            if table == 'kfreq':
                index = columns.index('n')
                expanded[table] = (columns, [tuple(value * copies if i == index else value
                                                 for i, value in enumerate(row)) for row in rows])
                continue
            fid_index = columns.index('fid')
            values = []
            for copy in range(copies):
                for row in rows:
                    value = list(row)
                    value[fid_index] += copy * stride
                    if table == 'file':
                        value[columns.index('path')] += '.spike-' + str(copy)
                    values.append(tuple(value))
            expanded[table] = (columns, values)
        result.append((expanded, {fid + copy * stride: text for copy in range(copies)
                                  for fid, text in tokens.items()}))
    return result


def publish(meter, table, columns, rows):
    if not rows:
        return
    width = len(columns)
    limit = meter.db.getlimit(sqlite3.SQLITE_LIMIT_VARIABLE_NUMBER) // width
    for offset in range(0, len(rows), limit):
        batch = rows[offset:offset + limit]
        sql = ('INSERT INTO ' + quote(table) + '(' + ','.join(map(quote, columns)) + ') VALUES '
               + ','.join('(' + ','.join('?' for _ in columns) + ')' for _ in batch))
        meter.execute(sql, [value for row in batch for value in row])


def fingerprint(meter, tables):
    # One compound read verifies every logical index row, including stale vocabulary,
    # overflow and postings. Verification never issues a query for each file.
    verified = {**tables, 'vlex': (['lang', 'term', 'n'], []),
                'ftsk_inst': (['term', 'doc', 'col', 'offset'], [])}
    width = max(len(columns) for columns, _ in verified.values())
    arms = []
    for table, (columns, _) in verified.items():
        select = ["'" + table + "'", *map(quote, columns), *['NULL'] * (width - len(columns))]
        arms.append('SELECT ' + ','.join(select) + ' FROM ' + quote(table))
    rows = meter.execute(' UNION ALL '.join(arms)).fetchall()
    values = [[['blob', value.hex()] if isinstance(value, bytes) else value for value in row]
              for row in rows]
    normalized = sorted(json.dumps(row, separators=(',', ':')) for row in values)
    return {'rows': len(rows), 'sha256': hashlib.sha256(json.dumps(normalized).encode()).hexdigest()}


def trial(captures, startup, mode, number, trace=False):
    start = time.process_time()
    result, phases, meters, copied_pages = [], {}, [], []
    verification_cpu = 0
    with tempfile.TemporaryDirectory(prefix='rerail-owner-spike-') as directory:
        directory = Path(directory)
        meter = seed = None
        if mode == 'seed':
            seed = Meter(directory / 'seed.sqlite', trace)
            meters.append(seed)
            for sql in startup:
                seed.execute(sql).fetchall()
            seed.phase = 'publication'
            seed.execute('BEGIN')
            for table in CATALOG:
                publish(seed, table, *captures[0][0][table])
            seed.execute('COMMIT')
        for domain, (tables, tokens) in enumerate(captures):
            if meter is None:
                meter = Meter(directory / f'owner-{domain}.sqlite', trace)
                meters.append(meter)
                if seed:
                    pages = []
                    seed.db.backup(meter.db, progress=lambda _status, _remaining, total: pages.append(total))
                    copied_pages.append(max(pages))
                    # Main-schema backup does not copy connection settings or TEMP tables.
                    for sql in startup[:10]:
                        meter.execute(sql).fetchall()
                else:
                    for sql in startup:
                        meter.execute(sql).fetchall()
            meter.phase = 'publication'
            meter.execute('BEGIN')
            for table in CATALOG:
                if mode == 'fresh' or (mode == 'reuse' and domain == 0):
                    publish(meter, table, *tables[table])
            for table in TABLES:
                if table not in CATALOG:
                    publish(meter, table, *tables[table])
            publish(meter, 'ftsk', ['rowid', 'k'], list(tokens.items()))
            meter.execute('COMMIT')
            meter.phase = 'verification'
            before = time.process_time()
            result.append(fingerprint(meter, tables))
            verification_cpu += time.process_time() - before
            if mode == 'reuse' and domain < len(captures) - 1:
                meter.phase = 'retirement'
                # Existing foreign keys and FTS triggers maintain dependent data.
                # No new triggers change the accounting origin of engine work.
                meter.execute('BEGIN')
                meter.execute('DELETE FROM file')
                meter.execute('DELETE FROM kfreq')
                meter.execute('UPDATE vocabulary_predicate SET ready=0')
                meter.execute('COMMIT')
            if mode != 'reuse' or domain == len(captures) - 1:
                meter.close()
                meter = None
        if seed:
            seed.close()
        for meter in meters:
            for key, value in meter.by_phase.items():
                phases[key] = phases.get(key, 0) + value
    statements = sum(meter.statements for meter in meters)
    observed = sum(meter.trace_executions for meter in meters)
    if trace:
        assert observed >= statements
    cpu = (time.process_time() - start) * 1000
    return {'trial': number, 'mode': mode, 'engineStatements': statements,
            'sqliteInternalExecutions': observed - statements if trace else None,
            'traceExecutions': observed if trace else None, 'phases': phases,
            'cpuMs': cpu, 'workCpuMs': cpu - verification_cpu * 1000,
            'verificationCpuMs': verification_cpu * 1000,
            'backupCalls': len(copied_pages), 'copiedPages': copied_pages,
            'fingerprint': result}


def experiment(raw, output, copies):
    captures, startup = inputs(raw)
    captures = scale(captures, copies)
    trials = []
    modes = ['fresh', 'reuse', 'seed']
    for number in range(5):
        order = modes[number % 3:] + modes[:number % 3]
        if number % 2:
            order.reverse()
        pair = [trial(captures, startup, mode, number) for mode in order]
        assert all(row['fingerprint'] == pair[0]['fingerprint'] for row in pair)
        trials.extend(pair)
    counted = [trial(captures, startup, mode, -1, True) for mode in modes]
    assert all(row['fingerprint'] == counted[0]['fingerprint'] for row in counted)
    report = {'counted': counted, 'configuredCacheKiB': {'main': 1024, 'temp': 512},
              'originAccounting': 'Counted passes trace every SQLite execution. Each Meter call executes one engine statement; remaining callbacks are SQLite internal executions. Timed trials disable tracing. No new triggers are introduced. SQLite backup API calls are reported and included in CPU.',
              'format': 'rerail.post-c-storage-spike/v2', 'sqliteVersion': sqlite3.sqlite_version,
              'scope': 'Four SQL domains reconstructed from captured typed facts. All SQL setup, publication, retirement and verification count. Parsing, codec construction and rule execution are outside this component experiment. Verification CPU is reported separately. No complete-job saving is inferred.',
              'copiesPerSource': copies, 'syntheticReplication': copies != 1,
              'domains': len(captures), 'trials': trials}
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps([{key: value for key, value in row.items() if key != 'fingerprint'}
                      for row in counted], indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--raw', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--copies', type=int, default=1, help='Standalone publication stress; repeats real tapes at distinct paths')
    args = parser.parse_args()
    if args.copies < 1 or args.copies > 1024:
        parser.error('copies must be 1..1024')
    experiment(args.raw, args.output, args.copies)
