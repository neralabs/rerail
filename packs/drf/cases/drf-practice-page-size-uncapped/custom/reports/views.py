from rest_framework import generics
from rest_framework.pagination import LimitOffsetPagination

from api.pagination import DefaultPagination, SizedPages, SmallPages
from reports.models import Report
from reports.pagination import ExportPagination


class ReportList(generics.ListAPIView):
    queryset = Report.objects.order_by("id")
    pagination_class = SmallPages


class ReportExport(generics.ListAPIView):
    queryset = Report.objects.order_by("id")
    pagination_class = ExportPagination


class ReportArchive(generics.ListAPIView):
    queryset = Report.objects.order_by("id")
    pagination_class = DefaultPagination


class ReportPages(generics.ListAPIView):
    queryset = Report.objects.order_by("id")
    pagination_class = SizedPages


class ReportFeed(generics.ListAPIView):
    queryset = Report.objects.order_by("id")
    pagination_class = LimitOffsetPagination  # caught
