import requests
async def f():
    requests.get('https://example.com')

import requests
async def f():
    requests.get('https://example.com')  # noqa

import requests
async def f():
    requests.get('https://example.com')  # noqa: ASYNC210

import requests
async def f():
    requests.get('https://example.com')  # NOQA: ASYNC210

import requests
async def f():
    requests.get('https://example.com')  # noqa: E501
