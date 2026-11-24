from rest_framework.pagination import LimitOffsetPagination, PageNumberPagination


class LooseLimitOffset(LimitOffsetPagination):
    default_limit = 5


class LoosePages(PageNumberPagination):
    page_size_query_param = "page_size"
