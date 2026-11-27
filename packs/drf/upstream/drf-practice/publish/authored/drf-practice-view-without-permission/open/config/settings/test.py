# Test settings are not the project's: this default does not close the open project.
from .base import *  # noqa: F403

REST_FRAMEWORK = {
    "DEFAULT_PERMISSION_CLASSES": ["rest_framework.permissions.IsAuthenticated"],
}
