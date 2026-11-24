from config.settings.base import *  # noqa: F403

# Test settings are not the project's settings.
REST_FRAMEWORK["DEFAULT_PAGINATION_CLASS"] = "rest_framework.pagination.PageNumberPagination"  # noqa: F405
