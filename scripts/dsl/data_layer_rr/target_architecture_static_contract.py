"""Check scope and repeated declaration semantics outside the original witnesses."""
from pathlib import Path

from dto import canonical

CHECK = 'serverplan_target_architecture'


def cases(_module):
    forbidden = 'class BatchedStep: pass\n'
    return [
        dict(id='scope', kind='static', shape=False, sources={
            'l2070_batching/test_legacy.py': forbidden,
            'l2070_batching/tests/legacy.py': forbidden,
            'l2070_batching/test/legacy.py': forbidden,
            'l2070_batching/tmp/legacy.py': forbidden,
            'l2070_batching/legacy.txt': forbidden,
            'other/legacy.py': forbidden,
        }),
        dict(id='last-class-definition', kind='static', shape=True, sources={
            'l2070_batching/planning_contract.py':
                'from enum import StrEnum\n'
                'class SessionContextScope(StrEnum):\n'
                '    NONE = "none"\n'
                '    STATEMENT = "statement"\n'
                '    TRANSACTION = "transaction"\n'
                'class SessionContextScope:\n'
                '    NONE = "none"\n',
        }),
        dict(id='last-union-assignment', kind='static', shape=True, sources={
            'l2070_batching/physical_program.py':
                'PhysicalTransportRegion = SqlStatementRegion | ClosedApplicationRegion | ExternalIoRegion\n'
                'PhysicalTransportRegion = SqlStatementRegion\n',
        }),
    ]


def exercise(module, case, _temporary):
    result = module.scan_sources({Path(name): text for name, text in case['sources'].items()},
                                 include_required_shape=case['shape'])
    return canonical(result)


def install(backend, module, detectors=False):
    from target_architecture_port import scan

    def scan_sources(sources, *, include_required_shape=True):
        return scan(backend, module, dict(sources=sources, include_required_shape=include_required_shape))

    backend.replace(module, 'scan_sources', scan_sources)


if __name__ == '__main__':
    from contract_runner import main

    main(CHECK, 'rerail.target-architecture-static-contract/v1', cases, exercise, install,
         ['target_architecture_static_contract.py', 'target_architecture_port.py'])
