from django.core.exceptions import ImproperlyConfigured

from .base import *  # noqa: F403


def check_settings():
    if SECRET_KEY == "insecure-dev-key":  # noqa: F405
        raise ImproperlyConfigured("Set SECRET_KEY.")
