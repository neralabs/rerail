import os
async def f():
    os.popen('ls')

import os
async def f():
    os.popen('ls')  # noqa

import os
async def f():
    os.popen('ls')  # noqa: ASYNC220

import os
async def f():
    os.popen('ls')  # NOQA: ASYNC220

import os
async def f():
    os.popen('ls')  # noqa: E501
