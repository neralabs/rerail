"""Not a settings module: one DEBUG flag for a script."""

import os

DEBUG = os.getenv("DEBUG", "1") == "1"
