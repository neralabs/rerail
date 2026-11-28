# A settings module: it assigns INSTALLED_APPS. Imports between settings modules are left alone.
from myproject.settings.helpers import env_bool

DEBUG = env_bool("DEBUG")
SITE_URL = "https://example.com"
INSTALLED_APPS = ["django.contrib.auth", "shop"]

from .local import *  # noqa: E402,F403
