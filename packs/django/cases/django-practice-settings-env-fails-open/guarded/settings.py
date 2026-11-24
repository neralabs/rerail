"""Fallbacks that cannot reach production, safe defaults, and reads that are not env reads."""

import configparser
import os

from django.core.exceptions import ImproperlyConfigured

config = configparser.RawConfigParser()
config.read(["/etc/guarded.cfg"])

SECRET_KEY = os.environ.get("SECRET_KEY", "change-me")
if SECRET_KEY == "change-me":
    raise ImproperlyConfigured("Set SECRET_KEY.")

DEBUG = os.environ.get("DEBUG", "False") == "True"
if DEBUG:
    ALLOWED_HOSTS = os.environ.get("ALLOWED_HOSTS", "*").split(",")
else:
    ALLOWED_HOSTS = os.environ["ALLOWED_HOSTS"].split(",")

DEBUG = config.getboolean("django", "debug", fallback=True)
