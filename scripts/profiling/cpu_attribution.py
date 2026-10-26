"""Subtract nested thread CPU. Keep shared and unknown work explicitly unattributed."""
import collections


def summarize(events):
    threads = collections.defaultdict(list)
    missing = overlaps = overdraw = unplaced = 0
    for event in events:
        if not all(k in event for k in ['thread','startUnixUs','wallNs']):
            missing += event.get('cpuNs') is None
            unplaced += 1
            continue
        threads[event['thread']].append(event)
    groups = {}
    for rows in threads.values():
        stack = []
        nodes = []
        for event in sorted(rows, key=lambda e: (e['startUnixUs'], -e['wallNs'])):
            start = event['startUnixUs'] * 1000
            end = start + event['wallNs']
            while stack and start >= stack[-1]['end']:
                stack.pop()
            # Timestamp rounding can move a nested endpoint by one microsecond.
            while stack and end > stack[-1]['end'] + 2000:
                overlaps += 1
                stack.pop()
            parent = stack[-1] if stack else None
            rule, clause = event.get('rule', ''), event.get('clause', '')
            if not rule and not clause and parent:
                rule, clause = parent['rule'], parent['clause']
            node = dict(event=event, end=end, childrenCpu=0, rule=rule, clause=clause)
            if parent:
                parent['childrenCpu'] += event.get('cpuNs') or 0
            stack.append(node)
            nodes.append(node)
        for node in nodes:
            event = node['event']
            cpu = event.get('cpuNs')
            if cpu is None:
                missing += 1
                continue
            if node['childrenCpu'] > cpu:
                overdraw += 1
            own = max(0, cpu - node['childrenCpu'])
            key = (event.get('runPhase', ''), node['rule'], node['clause'], event['phase'], event['name'])
            row = groups.setdefault(key, dict(runPhase=key[0], rule=key[1], clause=key[2],
                                             phase=key[3], name=key[4], calls=0,
                                             selfCpuNs=0, inclusiveCpuNs=0))
            row['calls'] += 1
            row['selfCpuNs'] += own
            row['inclusiveCpuNs'] += cpu
    output = sorted(groups.values(), key=lambda r: -r['selfCpuNs'])
    return dict(observedCpuNs=sum(r['selfCpuNs'] for r in output),
                unattributedCpuNs=sum(r['selfCpuNs'] for r in output if not r['rule'] or not r['clause']),
                missingCpuEvents=missing, unplacedEvents=unplaced, partialOverlapEvents=overlaps,
                cpuOverdrawEvents=overdraw, rows=output,
                accounting='Measured spans only. Self CPU excludes nested spans on the same thread. '
                           'Untraced work and ambiguous overlaps are not inferred.')
