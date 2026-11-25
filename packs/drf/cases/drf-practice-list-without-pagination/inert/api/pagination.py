from rest_framework import pagination
from rest_framework.pagination import CursorPagination, PageNumberPagination


# Derives from PageNumberPagination and sets no page_size: pages nothing unless ?page_size= is sent.
class ClientSizedPages(PageNumberPagination):
    page_size_query_param = "page_size"
    max_page_size = 100


class SizedPages(PageNumberPagination):
    page_size = 50


class EventFeed(CursorPagination):
    page_size = 25
    ordering = "-id"


class ConfiguredPages(pagination.LimitOffsetPagination):
    def __init__(self):
        self.default_limit = 20


class ComputedPages(PageNumberPagination):
    def get_page_size(self, request):
        return 30
