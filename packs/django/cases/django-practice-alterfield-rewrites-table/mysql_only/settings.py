# The project runs on MySQL only: PostgreSQL's rewrite rules do not apply.
DATABASES = {
    "default": {
        "ENGINE": "django.db.backends.mysql",
        "NAME": "shop",
    },
}
