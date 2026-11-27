"""django-environ: the env instance and its schema live in another module."""

from environish.env import env

DEBUG = env("DEBUG")  # caught
SECRET_KEY = env("SECRET_KEY")
ALLOWED_HOSTS = env.list("ALLOWED_HOSTS", default=["*"])  # caught
CSRF_TRUSTED_ORIGINS = env.list("CSRF_TRUSTED_ORIGINS", default=["*"])
SITE_DEBUG = env("SITE_DEBUG")
