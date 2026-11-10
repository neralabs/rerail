"""Audit the complete backend scanner against the fixed original-test inventory."""
from recorded_admission import validate,admit
from recorded_admission import pack as _pack,main as _main


def pack(work):return _pack(work,'rerail.backend-types-admission/v1')


if __name__=='__main__':_main('rerail.backend-types-admission/v1')
