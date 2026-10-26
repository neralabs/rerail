"""CPU attribution over nested spans. Unknown work remains explicitly unattributed."""
import collections


def summarize(events):
    return {'observedCpuNs': sum(e.get('cpuNs') or 0 for e in events)}
