INSTALLED_APPS = ["rest_framework", "api", "reports"]

API_MAX_PAGE_SIZE = 500

REST_FRAMEWORK = {
    # A project class: the rule judges the class (api/pagination.py), not this setting.
    "DEFAULT_PAGINATION_CLASS": "api.pagination.DefaultPagination",
    "PAGE_SIZE": 25,
}
