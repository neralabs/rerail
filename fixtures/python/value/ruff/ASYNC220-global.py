import os

MODE = os.P_WAIT
NOWAIT = os.P_NOWAIT


async def declares_global():
    global MODE
    os.spawnl(MODE, "a")


async def reads_module():
    os.spawnl(MODE, "a")
    os.spawnl(NOWAIT, "a")


async def rebinds_global():
    global NOWAIT
    NOWAIT = os.P_WAIT
    os.spawnl(NOWAIT, "a")


async def declares_other():
    global OTHER
    os.spawnl(MODE, "a")


async def locals_and_literals():
    m = os.P_WAIT
    os.spawnl(m, "a")
    n = os.P_NOWAIT
    os.spawnl(n, "a")
    os.spawnl(os.P_WAIT, "a")
    os.spawnl(os.P_NOWAIT, "a")
    os.spawnl(unknown, "a")
    os.spawnl(late, "a")
    late = os.P_WAIT


async def outer():
    global MODE

    async def inner():
        os.spawnl(MODE, "a")

    os.spawnl(MODE, "a")


def not_async():
    global MODE
    os.spawnl(MODE, "a")
