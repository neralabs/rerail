"""The importer checks the key only inside a function, which settings loading never calls."""

import os

SECRET_KEY = os.environ.get("SECRET_KEY", "insecure-dev-key")  # caught
INSTALLED_APPS = ["django.contrib.auth"]
