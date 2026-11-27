import environ

env = environ.Env(
    DEBUG=(bool, True),
    SECRET_KEY=(str, ""),
    SITE_DEBUG=(bool, True),
)
