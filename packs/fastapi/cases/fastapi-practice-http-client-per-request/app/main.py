from contextlib import asynccontextmanager

import httpx
from fastapi import FastAPI, Request
from starlette.middleware.base import BaseHTTPMiddleware

from .routes import router

shared = httpx.AsyncClient(timeout=10)


@asynccontextmanager
async def lifespan(app: FastAPI):
    async with httpx.AsyncClient() as client:
        app.state.http = client
        yield


app = FastAPI(lifespan=lifespan)
app.include_router(router)


@app.middleware("http")
async def audit(request: Request, call_next):
    response = await call_next(request)
    async with httpx.AsyncClient() as client:  # caught
        await client.post("https://audit.example.com", json={"path": request.url.path})
    return response


@app.middleware("http")
async def audit_shared(request: Request, call_next):
    response = await call_next(request)
    await request.app.state.http.post("https://audit.example.com", json={})
    return response


class Tracing(BaseHTTPMiddleware):
    async def dispatch(self, request, call_next):
        client = httpx.Client()  # caught
        client.post("https://trace.example.com")
        client.close()
        return await call_next(request)


app.add_middleware(Tracing)
