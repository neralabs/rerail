import os

# PostgreSQL in production; MySQL is supported too, so the rule still reads this project's migrations.
DATABASES = {
    "default": {
        "ENGINE": os.environ.get("DB_ENGINE", "django.db.backends.postgresql"),
        "NAME": "shop",
    },
    "legacy": {
        "ENGINE": "django.db.backends.mysql",
        "NAME": "shop_legacy",
    },
}
