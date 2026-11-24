# The app imported from another module: still the app built with lifespan=.
from app.main import app


@app.on_event("startup")  # caught
def load_models() -> None: ...
