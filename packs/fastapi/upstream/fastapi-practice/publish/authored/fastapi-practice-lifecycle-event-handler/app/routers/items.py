from fastapi import APIRouter

# A router included into an app with a lifespan: FastAPI merges the router's own default lifespan into the app's, so
# these still run, until someone moves them.
router = APIRouter(prefix="/items", on_startup=[lambda: None])  # caught


@router.on_event("startup")  # caught
async def prime() -> None: ...


@router.get("/")
async def list_items() -> list[str]:
    return []
