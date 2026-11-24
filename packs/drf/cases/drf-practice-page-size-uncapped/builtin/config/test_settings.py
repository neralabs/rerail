from config.settings import *  # noqa: F403

REST_FRAMEWORK["DEFAULT_PAGINATION_CLASS"] = "rest_framework.pagination.LimitOffsetPagination"
