from contextlib import asynccontextmanager

from fastapi import APIRouter, FastAPI


@asynccontextmanager
async def lifespan(app: FastAPI):
    yield


app = FastAPI(lifespan=lifespan)
router = APIRouter(lifespan=lifespan)
app.include_router(router)
