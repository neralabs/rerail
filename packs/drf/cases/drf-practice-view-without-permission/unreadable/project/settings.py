import environ

env = environ.Env()

# The default comes from the environment: the rule cannot tell it is AllowAny, so it stays silent.
REST_FRAMEWORK = {
    "DEFAULT_PERMISSION_CLASSES": env.list("API_PERMISSION_CLASSES", default=["rest_framework.permissions.AllowAny"]),
}
