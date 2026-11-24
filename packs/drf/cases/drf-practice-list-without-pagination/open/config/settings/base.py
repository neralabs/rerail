# PAGE_SIZE without DEFAULT_PAGINATION_CLASS: DRF paginates nothing (its check rest_framework.W001 says so).
REST_FRAMEWORK = {
    "DEFAULT_AUTHENTICATION_CLASSES": ["rest_framework.authentication.SessionAuthentication"],
    "PAGE_SIZE": 50,
}
