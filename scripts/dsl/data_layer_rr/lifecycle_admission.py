"""Audit lifecycle contracts through the shared, fixed-inventory admission gate."""
from recorded_admission import validate,admit
from recorded_admission import pack as _pack,main as _main


def pack(work):return _pack(work,'rerail.lifecycle-admission/v1')


if __name__=='__main__':_main('rerail.lifecycle-admission/v1')
