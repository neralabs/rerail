"""Audit API-import contracts through the shared fixed-inventory admission gate."""
from recorded_admission import validate,admit
from recorded_admission import pack as _pack,main as _main


def pack(work):return _pack(work,'rerail.api-imports-admission/v1')


if __name__=='__main__':_main('rerail.api-imports-admission/v1')
