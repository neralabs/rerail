import environ

env = environ.Env()

DATABASES = {
    "default": {
        "ENGINE": "django.db.backends.postgresql",
        "NAME": "shop",
        "ATOMIC_REQUESTS": True,
    },
    "replica": {
        "ENGINE": "django.db.backends.postgresql",
        "NAME": "shop",
        "ATOMIC_REQUESTS": env.bool("REPLICA_ATOMIC", default=False),
    },
}

# Not a Django setting: ATOMIC_REQUESTS lives in each database's dict.
ATOMIC_REQUESTS = True
