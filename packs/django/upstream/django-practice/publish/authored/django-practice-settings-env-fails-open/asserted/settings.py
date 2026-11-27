"""An assert is not a refusal: `python -O` strips it, and the key stays."""

import os

DEBUG = False
SECRET_KEY = os.environ.get("SECRET_KEY", "x")  # caught
assert SECRET_KEY != "x" or DEBUG
