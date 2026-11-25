from fastapi import FastAPI

from app.lifespan import lifespan
from app.plugins import install_metrics, install_tracing
from app.routers.items import router as items_router

# The app has a lifespan: FastAPI runs only it, never the app's startup/shutdown handlers.
app = FastAPI(title="shop", lifespan=lifespan)
app.include_router(items_router)
install_metrics(app)
install_tracing(app=app)


@app.on_event("startup")  # caught
async def warm_cache() -> None: ...


async def close_pool() -> None: ...


app.add_event_handler("shutdown", close_pool)  # caught
app.on_event("shutdown")(close_pool)  # caught
app.router.add_event_handler("startup", close_pool)  # caught
app.add_event_handler(event_type="startup", func=close_pool)  # caught

# Gap, not reported: appended to the router's list by hand.
app.router.on_startup.append(close_pool)

# Both at once: the on_startup list is ignored.
both = FastAPI(
    lifespan=lifespan,
    on_startup=[close_pool],  # caught
)
