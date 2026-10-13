import httpx
async def f():
    client = httpx.Client()
    client.get('https://example.com')

import httpx
async def f():
    client = httpx.Client()
    client.get('https://example.com')  # noqa

import httpx
async def f():
    client = httpx.Client()
    client.get('https://example.com')  # noqa: ASYNC212

import httpx
async def f():
    client = httpx.Client()
    client.get('https://example.com')  # NOQA: ASYNC212

import httpx
async def f():
    client = httpx.Client()
    client.get('https://example.com')  # noqa: E501
