from .base import *  # noqa: F403

# The production settings set the default: every view without its own permission classes requires a login.
REST_FRAMEWORK["DEFAULT_PERMISSION_CLASSES"] = ("rest_framework.permissions.IsAuthenticated",)
