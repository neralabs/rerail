"""A base module whose deployed importers refuse or replace its fallbacks."""

import os

# production.py refuses DEBUG on, staging.py sets it: silent.
DEBUG = os.environ.get("DEBUG", "1") == "1"
# Both refuse the development key: silent.
SECRET_KEY = os.environ.get("SECRET_KEY", "insecure-dev-key")
# production.py refuses "*", staging.py does neither: staging runs with it.
ALLOWED_HOSTS = os.environ.get("ALLOWED_HOSTS", "*").split(",")  # caught
INSTALLED_APPS = ["django.contrib.auth"]
