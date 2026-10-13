import subprocess
async def f():
    subprocess.run(['ls'])

import subprocess
async def f():
    subprocess.run(['ls'])  # noqa

import subprocess
async def f():
    subprocess.run(['ls'])  # noqa: ASYNC221

import subprocess
async def f():
    subprocess.run(['ls'])  # NOQA: ASYNC221

import subprocess
async def f():
    subprocess.run(['ls'])  # noqa: E501
