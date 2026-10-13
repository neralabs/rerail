import os
async def f():
    os.wait()

import os
async def f():
    os.wait()  # noqa

import os
async def f():
    os.wait()  # noqa: ASYNC222

import os
async def f():
    os.wait()  # NOQA: ASYNC222

import os
async def f():
    os.wait()  # noqa: E501
