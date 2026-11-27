"""A development settings module: its fallbacks never reach production."""

import os

from .common import *  # noqa: F403

DEBUG = os.environ.get("DEBUG", "1") == "1"
SECRET_KEY = os.environ.get("SECRET_KEY", "local-dev-key")
