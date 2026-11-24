from rest_framework import generics, mixins, viewsets
from rest_framework.response import Response
from vendor.api import ListAPIView as VendorListAPIView

from shop.api.base import PagedViewSet, ProjectListView, SelfPagingViewSet, paginator
from shop.api.serializers import InvoiceSerializer, OrderSerializer, TagSerializer, VersionSerializer
from shop.models import Customer, Invoice, Order, Tag


class OrderViewSet(viewsets.ModelViewSet):  # caught
    queryset = Order.objects.select_related("customer")
    serializer_class = OrderSerializer


class InvoiceList(generics.ListAPIView):  # caught
    serializer_class = InvoiceSerializer

    def get_queryset(self):
        return Invoice.objects.filter(order__customer__user=self.request.user)


class OrderHistory(ProjectListView):  # caught
    queryset = Order.objects.all()
    serializer_class = OrderSerializer


class TagViewSet(mixins.ListModelMixin, viewsets.GenericViewSet):  # caught
    serializer_class = TagSerializer

    def get_queryset(self):
        return self.request.user.tags.all()


class OrderFeed(viewsets.ReadOnlyModelViewSet):  # caught
    queryset = Order.objects.all()
    serializer_class = OrderSerializer

    def list(self, request, *args, **kwargs):
        response = super().list(request, *args, **kwargs)
        response["Cache-Control"] = "no-store"
        return response


class InvoiceArchive(generics.ListCreateAPIView):  # caught
    queryset = Invoice.objects.none()
    serializer_class = InvoiceSerializer

    def get_queryset(self):
        queryset = Invoice.objects.filter(order__customer__user=self.request.user)
        if self.request.query_params.get("open"):
            queryset = queryset.filter(paid=False)
        return queryset


# Near misses.
class PagedOrders(PagedViewSet):
    queryset = Order.objects.all()
    serializer_class = OrderSerializer


class SmallOrderList(generics.ListAPIView):
    queryset = Order.objects.all()
    serializer_class = OrderSerializer
    pagination_class = None


class RecentOrders(generics.ListAPIView):
    queryset = Order.objects.order_by("-created")[:50]
    serializer_class = OrderSerializer


class LatestInvoices(generics.ListAPIView):
    serializer_class = InvoiceSerializer

    def get_queryset(self):
        return Invoice.objects.filter(order__customer__user=self.request.user).order_by("-id")[:20]


class TrimmedInvoices(generics.ListAPIView):
    serializer_class = InvoiceSerializer

    def get_queryset(self):
        queryset = Invoice.objects.filter(order__customer__user=self.request.user)
        queryset = queryset[:100]
        return queryset


class OrderSearch(generics.ListAPIView):
    queryset = Order.objects.all()
    serializer_class = OrderSerializer

    def list(self, request, *args, **kwargs):
        page = self.paginate_queryset(self.filter_queryset(self.get_queryset()))
        return self.get_paginated_response(self.get_serializer(page, many=True).data)


class OrderCount(generics.ListAPIView):
    queryset = Order.objects.all()

    def get(self, request, *args, **kwargs):
        return Response({"count": self.get_queryset().count()})


class StatsViewSet(viewsets.ViewSet):
    def list(self, request):
        return Response({"orders": Order.objects.count()})


class OwnPagingOrders(SelfPagingViewSet):
    queryset = Order.objects.all()
    serializer_class = OrderSerializer


class VersionList(generics.ListAPIView):
    serializer_class = VersionSerializer

    def get_queryset(self):
        return [{"name": "1.0"}, {"name": "2.0"}]


class OrderCreate(generics.CreateAPIView):
    queryset = Order.objects.all()
    serializer_class = OrderSerializer


class OrderDetail(generics.RetrieveUpdateAPIView):
    queryset = Order.objects.all()
    serializer_class = OrderSerializer


class VendorOrders(VendorListAPIView):
    queryset = Order.objects.all()


class PreferenceViewSet(viewsets.ModelViewSet):
    queryset = Tag.objects.all()
    serializer_class = TagSerializer
    pagination_disabled = True


class TaskList(generics.ListAPIView):
    queryset = Order.objects.all()
    serializer_class = OrderSerializer

    def filter_queryset(self, queryset):
        return paginator(queryset.order_by("-id"), self.request)


class ProfileViewSet(viewsets.ModelViewSet):
    queryset = Customer.objects.all()
    serializer_class = OrderSerializer


class InvoiceBoardViewSet(viewsets.ModelViewSet):  # caught
    queryset = Invoice.objects.all()
    serializer_class = InvoiceSerializer


class RoutedOrders(viewsets.ReadOnlyModelViewSet):  # caught
    queryset = Order.objects.all()
    serializer_class = OrderSerializer
