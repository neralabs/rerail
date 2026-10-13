import os.path
async def f():
    os.path.exists('data.txt')

import os.path
async def f():
    os.path.exists('data.txt')  # noqa

import os.path
async def f():
    os.path.exists('data.txt')  # noqa: ASYNC240

import os.path
async def f():
    os.path.exists('data.txt')  # NOQA: ASYNC240

import os.path
async def f():
    os.path.exists('data.txt')  # noqa: E501
