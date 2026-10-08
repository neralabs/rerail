# target-version: py314
from threading import Thread


def reads_the_module_first():
    from threading import Thread

    local: Thread
