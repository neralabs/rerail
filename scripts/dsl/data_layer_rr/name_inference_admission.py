"""Admit name-inference contracts against the fixed original assertion inventory."""
from recorded_admission import validate,admit
from recorded_admission import pack as _pack,main as _main


def pack(work):return _pack(work,'rerail.name-inference-admission/v1')


if __name__=='__main__':_main('rerail.name-inference-admission/v1')
