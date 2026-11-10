"""Audit and admit the recorded removed contracts through the shared admission gate."""
from recorded_admission import validate, admit
from recorded_admission import pack as _pack, main as _main


def pack(work): return _pack(work,'rerail.removed-admission/v1')


if __name__=='__main__': _main('rerail.removed-admission/v1')
