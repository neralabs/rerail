"""Audit and admit the complete blanket-bypass rule contracts."""
from recorded_admission import validate,admit
from recorded_admission import pack as _pack,main as _main


def pack(work):return _pack(work,'rerail.blanket-admission/v1')


if __name__=='__main__':_main('rerail.blanket-admission/v1')
