import os

CORS_ALLOW_ORIGIN = os.getenv("CORS_ALLOW_ORIGIN", "*").split(";")
