"""A base module every settings module of the run star-imports (label-studio's shape)."""

from labelish.core.utils.params import get_bool_env, get_env, get_env_list

# Overridden with a False default by the one module that imports this one: not reported here.
DEBUG = get_bool_env("DEBUG", True)
ALLOWED_HOSTS = get_env_list("ALLOWED_HOSTS", default=["*"])  # caught
CSRF_TRUSTED_ORIGINS = get_env_list("CSRF_TRUSTED_ORIGINS", default=["*"])
SECRET_KEY = get_env("SECRET_KEY")
INSTALLED_APPS = ["django.contrib.auth"]
