"""Settings with env helpers defined in the module itself (healthchecks' shape)."""

import os
from pathlib import Path

from django.core.exceptions import ImproperlyConfigured

BASE_DIR = Path(__file__).resolve().parent.parent


def envbool(s, default):
    v = os.getenv(s, default=default)
    if v not in ("", "True", "False"):
        raise ImproperlyConfigured(f"Unexpected value {s}={v}")
    return v == "True"


def envsecret(s, default=None):
    if secret_path := os.getenv(s + "_FILE"):
        return Path(secret_path).read_text().strip()
    return os.getenv(s, default)


SECRET_KEY = envsecret("SECRET_KEY", "---")  # caught
DEBUG = envbool("DEBUG", "True")  # caught
TEMPLATE_DEBUG = envbool("TEMPLATE_DEBUG", "True")
REGISTRATION_OPEN = envbool("REGISTRATION_OPEN", "True")
SITE_NAME = envsecret("SITE_NAME", "Mychecks")
ALLOWED_HOSTS = os.getenv("ALLOWED_HOSTS", "localhost,127.0.0.1").split(",")
INSTALLED_APPS = ["django.contrib.auth", "hc.api"]
