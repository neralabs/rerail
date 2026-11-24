import environ

env = environ.Env()

# Two modules import this one and only one overrides DEBUG: the other runs with this default.
DEBUG = env.bool("DEBUG", default=True)  # caught
SECRET_KEY = env("SECRET_KEY")
