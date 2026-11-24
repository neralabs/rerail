"""A settings module in a `dev/` package: development only."""

import os

DEBUG = os.environ.get("DEBUG", "True") == "True"
SECRET_KEY = os.environ.get("SECRET_KEY", "dev-only-key")
ALLOWED_HOSTS = os.environ.get("ALLOWED_HOSTS", "*").split(",")
