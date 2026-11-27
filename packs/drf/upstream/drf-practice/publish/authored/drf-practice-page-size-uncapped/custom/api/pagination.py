from django.conf import settings
from rest_framework import pagination
from rest_framework.exceptions import ValidationError
from rest_framework.pagination import CursorPagination, LimitOffsetPagination, PageNumberPagination


class DefaultPagination(LimitOffsetPagination):  # caught
    default_limit = 25


class CappedLimitOffset(LimitOffsetPagination):
    max_limit = 100


class GenerousLimitOffset(pagination.LimitOffsetPagination):
    # A cap, however large, is a decision: left alone.
    max_limit = 1_000_000


class InheritsCap(CappedLimitOffset):
    default_limit = 10


class ExplicitNoCap(CappedLimitOffset):  # caught
    max_limit = None


class ConfiguredLimitOffset(LimitOffsetPagination):
    def __init__(self):
        self.max_limit = settings.API_MAX_PAGE_SIZE


class ClampedLimitOffset(LimitOffsetPagination):
    def get_limit(self, request):
        limit = super().get_limit(request)
        return min(limit or 25, 200)


class PassThroughLimitOffset(LimitOffsetPagination):  # caught
    def get_limit(self, request):
        if request.query_params.get("all") == "1":
            return None
        return super().get_limit(request)


class ValidatedLimitOffset(LimitOffsetPagination):
    def get_limit(self, request):
        if int(request.query_params.get("limit", 0)) > 500:
            raise ValidationError("limit too large")
        return super().get_limit(request)


class FixedLimitOffset(LimitOffsetPagination):
    # No limit parameter: the client cannot choose the page size.
    limit_query_param = None


class SizedPages(pagination.PageNumberPagination):  # caught
    page_size = 20
    page_size_query_param = "page_size"


class SizedCappedPages(PageNumberPagination):
    page_size_query_param = "page_size"
    max_page_size = settings.API_MAX_PAGE_SIZE


class FixedPages(PageNumberPagination):
    page_size = 20


class NoSizeParam(SizedPages):
    page_size_query_param = None


class FeedCursor(CursorPagination):  # caught
    ordering = "-created"
    page_size_query_param = "size"


class CappedFeedCursor(CursorPagination):
    ordering = "-created"
    page_size_query_param = "size"
    max_page_size = 200


# Only a base: the classes the views use set their own cap.
class AbstractSizedPages(PageNumberPagination):
    page_size_query_param = "page_size"


class SmallPages(AbstractSizedPages):
    max_page_size = 50


class UnlimitedOnRequest(PageNumberPagination):  # caught
    page_size_query_param = "page_size"

    def get_page_size(self, request):
        # emulate "unlimited" page_size
        if request.query_params.get(self.page_size_query_param) == "-1":
            return 1000000
        return super().get_page_size(request)


class ClampedPageSize(PageNumberPagination):
    page_size_query_param = "page_size"

    def get_page_size(self, request):
        return min(super().get_page_size(request) or 20, 100)


class PropertyCap(LimitOffsetPagination):
    @property
    def max_limit(self):
        return settings.API_MAX_PAGE_SIZE


class PlainPaginator:
    # Not a DRF pagination class.
    page_size_query_param = "page_size"


# paginate_queryset() clamps the client's limit itself.
class ClampedInPaginate(LimitOffsetPagination):
    def paginate_queryset(self, queryset, request, view=None):
        self.limit = min(self.get_limit(request) or 50, 500)
        self.offset = self.get_offset(request)
        self.count = self.get_count(queryset)
        self.request = request
        return list(queryset[self.offset:self.offset + self.limit])


class RejectingPages(PageNumberPagination):
    page_size_query_param = "size"

    def paginate_queryset(self, queryset, request, view=None):
        if int(request.query_params.get("size", 0)) > 200:
            raise ValidationError("size too large")
        return super().paginate_queryset(queryset, request, view)


# A paginate_queryset() that only logs: the client's ?limit= still goes through.
class LoggingLimitOffset(LimitOffsetPagination):  # caught
    def paginate_queryset(self, queryset, request, view=None):
        print("paginating")
        return super().paginate_queryset(queryset, request, view)
