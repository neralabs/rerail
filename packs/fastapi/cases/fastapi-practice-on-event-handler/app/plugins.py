from fastapi import FastAPI


def install_metrics(app):
    # Called with app.main's app, built with lifespan=: never runs.
    @app.on_event("startup")  # caught
    async def start_metrics() -> None: ...

    @app.on_event("shutdown")  # caught
    async def stop_metrics() -> None: ...


def install_tracing(app: FastAPI) -> None:
    app.add_event_handler("startup", start_tracing)  # caught


def start_tracing() -> None: ...


def install_unused(app):
    # Never called: an unannotated parameter with a literal event is still FastAPI's registration (deprecated).
    @app.on_event("startup")  # caught
    async def start() -> None: ...
