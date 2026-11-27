"""Safe defaults, and a literal SECRET_KEY that Bandit's S105 reports."""

import os
import secrets

from django.core.management.utils import get_random_secret_key

SECRET_KEY = "a-literal-key-bandit-reports"
SECRET_KEY = os.environ.get("SECRET_KEY", "")
SECRET_KEY = os.environ.get("SECRET_KEY", get_random_secret_key())
SECRET_KEY = os.getenv("SECRET_KEY") or secrets.token_urlsafe(50)
DEBUG = os.getenv("DEBUG", "no").lower() in ("yes", "true")
DEBUG = os.getenv("DEBUG", False)
DEBUG = os.environ.get("DEBUG", "True") != "True"
ALLOWED_HOSTS = os.getenv("ALLOWED_HOSTS", "").split(",")
ALLOWED_HOSTS = os.getenv("ALLOWED_HOSTS", ".example.com").split(",")
