"""DEBUG read as a str and used as it is, or through bool(): any non-empty str is true, "False" and "0" too."""

import os
from distutils.util import strtobool

import environ
from decouple import config

env = environ.Env()


def envflag(name, default):
    return os.getenv(name, default).lower() in ("1", "true")


DEBUG = os.getenv("DEBUG", "False")  # caught
DEBUG = os.environ.get("DJANGO_DEBUG", "0")  # caught
DEBUG = config("DEBUG", default="False")  # caught
DEBUG = env.bool("DEBUG", default="False")  # caught
DEBUG = env.str("DEBUG", "off")  # caught
DEBUG = bool(os.environ.get("DEBUG", "False"))  # caught
DEBUG = bool(os.getenv("DEBUG", "0"))  # caught
DEBUG = bool(config("DEBUG", default="no"))  # caught
DEBUG = os.environ.get("DEBUG") or "False"  # caught
DEBUG = str(os.getenv("DEBUG", False))  # caught

# Converted, compared or empty: false when the variable is unset.
DEBUG = os.getenv("DEBUG", "")
DEBUG = os.getenv("DEBUG")
DEBUG = config("DEBUG", default="False", cast=bool)
DEBUG = config("DEBUG", default="0", cast=int)
DEBUG = os.getenv("DEBUG", "False") == "True"
DEBUG = bool(int(os.getenv("DEBUG", "0")))
DEBUG = bool(os.getenv("DEBUG", ""))
DEBUG = strtobool(os.getenv("DEBUG", "False"))
DEBUG = bool(strtobool(os.environ.get("DEBUG", "no")))
DEBUG = os.environ.get("DEBUG", "False").lower() == "true"
DEBUG = os.environ.get("DEBUG", " FALSE ").strip().lower() in ("true", "1")
DEBUG = envflag("DEBUG", "False")
SECRET_KEY = os.environ["SECRET_KEY"]

# The same conversions with a true fallback.
DEBUG = config("DEBUG", default="1", cast=int)  # caught
DEBUG = bool(int(os.getenv("DEBUG", "1")))  # caught
DEBUG = strtobool(os.getenv("DEBUG", "yes"))  # caught
DEBUG = os.environ.get("DEBUG", " TRUE ").strip().lower() in ("true", "1")  # caught
DEBUG = envflag("DEBUG", "True")  # caught
