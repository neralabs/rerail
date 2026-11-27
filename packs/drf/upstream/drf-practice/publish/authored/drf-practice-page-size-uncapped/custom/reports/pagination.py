from api.base import ApiPagination
from api.pagination import AbstractSizedPages


class ExportPagination(ApiPagination):  # caught
    max_page_size = None


class ReportPagination(AbstractSizedPages):  # caught
    page_size = 10


class AuditPagination(ApiPagination):
    page_size = 50
