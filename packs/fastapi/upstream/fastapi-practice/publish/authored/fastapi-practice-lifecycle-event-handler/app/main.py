from fastapi import FastAPI

from app.lifespan import lifespan
from app.plugins import install_metrics, install_tracing
from app.routers.items import router as items_router

# The app has a lifespan: these handlers are dead, fastapi-practice-on-event-handler's case; not reported here.
app = FastAPI(title="shop", lifespan=lifespan)
app.include_router(items_router)
install_metrics(app)
install_tracing(app=app)


@app.on_event("startup")
async def warm_cache() -> None: ...


async def close_pool() -> None: ...


app.add_event_handler("shutdown", close_pool)
app.on_event("shutdown")(close_pool)
app.router.add_event_handler("startup", close_pool)
app.add_event_handler(event_type="startup", func=close_pool)

# Both at once: the on_startup list is ignored.
both = FastAPI(
    lifespan=lifespan,
    on_startup=[close_pool],
)
