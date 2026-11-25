"""DEBUG tested against a read with no fallback: unset, the read is None, which differs from every str."""

import os

DEBUG = os.environ.get("DJANGO_DEBUG") != "0"  # caught
DEBUG = os.getenv("DEBUG") not in ("0", "false", "False")  # caught
DEBUG = os.environ.get("DEBUG", "").lower() != "false"  # caught
DEBUG = not os.getenv("DISABLE_DEBUG")  # caught
DEBUG = os.getenv("PRODUCTION") is None  # caught
DEBUG = (os.getenv("ENVIRONMENT") or "dev") in ["dev", "local"]  # caught

# Off when the variable is unset.
DEBUG = os.environ.get("DEBUG") == "1"
DEBUG = os.getenv("DEBUG") in ("1", "true")
DEBUG = os.getenv("DEBUG", "0") != "0"
DEBUG = os.getenv("PRODUCTION") is not None
DEBUG = not os.getenv("PRODUCTION", "1")
DEBUG = os.getenv("DEBUG") and os.getenv("DEBUG") != "0"
SECRET_KEY = os.environ["SECRET_KEY"]
