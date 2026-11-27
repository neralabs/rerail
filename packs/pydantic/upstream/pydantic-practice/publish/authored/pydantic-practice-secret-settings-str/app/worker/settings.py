from app.core import config


# A settings class in a third module, deriving from the project's settings through a module attribute: its own
# fields are reported here, the inherited ones once, where they are declared.
class CelerySettings(config.Settings):
    RESULT_BACKEND_PASSWORD: str = ""  # caught
    RESULT_BACKEND_URL: str = "redis://localhost:6379/1"
