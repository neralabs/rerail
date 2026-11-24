from fastapi import FastAPI

test_app = FastAPI()


@test_app.on_event("startup")
async def start() -> None: ...
