import os

from .common import *  # noqa: F403

DEBUG = int(os.environ.get("DEBUG", 1)) == 1  # caught
