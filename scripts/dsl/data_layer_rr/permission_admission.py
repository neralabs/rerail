"""Audit and admit the recorded permission contracts through the shared admission gate."""
from recorded_admission import validate, admit
from recorded_admission import pack as _pack, main as _main


def pack(work): return _pack(work,'rerail.permission-admission/v1')


if __name__=='__main__': _main('rerail.permission-admission/v1')
