"""The helpers come from another module (saleor's shape)."""

import os
import warnings

from django.core.management.utils import get_random_secret_key

from envlib.conf.env import get_bool_from_env, get_list

DEBUG = get_bool_from_env("DEBUG", True)  # caught
SECRET_KEY = os.environ.get("SECRET_KEY")
if not SECRET_KEY and DEBUG:
    warnings.warn("SECRET_KEY not configured, using a random temporary key.", stacklevel=1)
    SECRET_KEY = get_random_secret_key()
ALLOWED_HOSTS = get_list(os.environ.get("ALLOWED_HOSTS", "localhost,127.0.0.1"))
ENABLE_SSL = get_bool_from_env("ENABLE_SSL", True)
