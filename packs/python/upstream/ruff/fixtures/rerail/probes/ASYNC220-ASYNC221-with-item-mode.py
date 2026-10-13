import os
async def f():
    with os.P_WAIT as m:
        os.spawnl(m, "x")
