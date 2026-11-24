"""The helper's own parameter default is the fallback when the call passes none."""

import os
from pathlib import Path


def env_flag(name, default="true"):
    return os.getenv(name, default).lower() in ("true", "1")


def from_file(name, default):
    path = Path("/run/config") / name
    return path.read_text() if path.exists() else default


DEBUG = env_flag("DEBUG")  # caught
SECRET_KEY = os.environ["SECRET_KEY"]
ALLOWED_HOSTS = from_file("hosts", "*").split(",")
