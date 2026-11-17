import requests as r
import os as o
from pathlib import Path as P
async def f12():
 r.get('url')
 o.system('cmd')
 o.spawnl(o.P_WAIT, 'x')
 o.waitpid(1, 0)
 P('x').open()
