"""os.environ reads under a settings/ package (plane's shape)."""

import os

from django.core.management.utils import get_random_secret_key

SECRET_KEY = os.environ.get("SECRET_KEY") or get_random_secret_key()
DEBUG = int(os.environ.get("DEBUG", "0"))
ALLOWED_HOSTS = os.environ.get("ALLOWED_HOSTS", "*").split(",")  # caught
CORS_ALLOWED_ORIGINS = os.environ.get("CORS_ALLOWED_ORIGINS", "*").split(",")
