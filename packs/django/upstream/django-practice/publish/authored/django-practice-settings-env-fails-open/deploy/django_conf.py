"""Not named settings.py, but it assigns Django settings, so it is a settings module."""

import os

INSTALLED_APPS = ["django.contrib.admin", "deploy"]
SECRET_KEY = os.getenv("DJANGO_SECRET_KEY", default="insecure")  # caught
DEBUG = os.environ.get("DJANGO_DEBUG", "true").lower() in ("true", "1", "yes")  # caught
ALLOWED_HOSTS = (  # caught
    os.environ.get("DJANGO_ALLOWED_HOSTS") or "localhost,*"
).split(",")
