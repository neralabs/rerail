"""DEBUG derived from another variable, an environment name whose fallback is a development one."""

import os

DEBUG = os.environ.get("APP_ENV", "development") == "development"  # caught
DEBUG = os.getenv("DJANGO_ENV", "Dev").lower() in ("dev", "local")  # caught

# The fallback names production: off.
DEBUG = os.environ.get("APP_ENV", "production") == "development"
DEBUG = os.getenv("DJANGO_ENV", "production") != "production"
SECRET_KEY = os.environ["SECRET_KEY"]
