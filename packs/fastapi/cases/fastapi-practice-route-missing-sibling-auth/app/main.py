from fastapi import Depends, FastAPI

from app.deps import get_current_user
from app.routes import accounts, items, keys, near_misses, orders, projects, reads, reports

app = FastAPI()
app.include_router(items.router)
app.include_router(orders.router)
app.include_router(keys.router)
app.include_router(accounts.router)
app.include_router(near_misses.small)
app.include_router(near_misses.mixed)
app.include_router(near_misses.guarded)
app.include_router(near_misses.plain)
app.include_router(reports.router, dependencies=[Depends(get_current_user)])
app.include_router(projects.router)
app.include_router(reads.router)
app.include_router(reads.admin)
