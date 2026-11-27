import os

from .common import *  # noqa: F403

DEBUG = os.environ.get("DEBUG", "True") == "True"
