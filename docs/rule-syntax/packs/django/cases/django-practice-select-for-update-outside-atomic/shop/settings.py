DATABASES = {
    "default": {
        "ENGINE": "django.db.backends.postgresql",
        "NAME": "shop",
        "ATOMIC_REQUESTS": False,
    }
}
# Turned on after the dict, by assignment: views run in a transaction.
DATABASES["default"]["ATOMIC_REQUESTS"] = True
