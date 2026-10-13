import time
async def f():
    time.sleep(1)

import time
async def f():
    time.sleep(1)  # noqa

import time
async def f():
    time.sleep(1)  # noqa: ASYNC251

import time
async def f():
    time.sleep(1)  # NOQA: ASYNC251

import time
async def f():
    time.sleep(1)  # noqa: E501
