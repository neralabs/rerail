from fastapi import FastAPI
from fastapi.responses import FileResponse

from app.routes import cases, files, near_misses
from app.shared import items, router as shared_router, search

app = FastAPI()
app.include_router(cases.router)
app.include_router(files.router)
app.include_router(near_misses.router)
app.include_router(shared_router.router)


# A single-page app's catch-all, declared before the health check.
@app.get("/{full_path:path}", include_in_schema=False)
def spa(full_path: str):
    return FileResponse("index.html")


@app.get("/health")  # caught
def health():
    return {"ok": True}
