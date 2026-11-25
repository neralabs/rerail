from django.core.exceptions import ImproperlyConfigured

from .base import *  # noqa: F403

if not SECRET_KEY or SECRET_KEY.startswith("insecure"):  # noqa: F405
    raise ImproperlyConfigured("Set SECRET_KEY.")
DEBUG = False
