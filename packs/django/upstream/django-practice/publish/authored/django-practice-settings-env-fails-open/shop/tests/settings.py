import os

DEBUG = os.environ.get("DEBUG", "True") == "True"
SECRET_KEY = os.environ.get("SECRET_KEY", "test-key")
ALLOWED_HOSTS = os.environ.get("ALLOWED_HOSTS", "*").split(",")
