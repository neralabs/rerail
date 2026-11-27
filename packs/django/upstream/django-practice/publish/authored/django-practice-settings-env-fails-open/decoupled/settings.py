"""python-decouple."""

from decouple import Csv, config

DEBUG = config("DEBUG", default=True, cast=bool)  # caught
SECRET_KEY = config("SECRET_KEY", "not-a-secret")  # caught
ALLOWED_HOSTS = config("ALLOWED_HOSTS", default="localhost", cast=Csv())
