from django.core.exceptions import ImproperlyConfigured

from .base import *  # noqa: F403

if SECRET_KEY == "insecure-dev-key":  # noqa: F405
    raise ImproperlyConfigured("Set SECRET_KEY.")
if DEBUG:  # noqa: F405
    raise ImproperlyConfigured("DEBUG must be off in production.")
if "*" in ALLOWED_HOSTS:  # noqa: F405
    raise ImproperlyConfigured("Set ALLOWED_HOSTS.")
