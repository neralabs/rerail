from ninja.pagination import LimitOffsetPagination as NinjaLimitOffset
from rest_framework import generics, pagination, viewsets
from rest_framework.decorators import action
from rest_framework.pagination import CursorPagination, LimitOffsetPagination, PageNumberPagination
from rest_framework.response import Response

from shop.models import Order, Product


class ProductList(generics.ListAPIView):
    queryset = Product.objects.order_by("id")
    pagination_class = LimitOffsetPagination  # caught


class OrderViewSet(viewsets.ModelViewSet):
    queryset = Order.objects.order_by("id")
    pagination_class = pagination.LimitOffsetPagination  # caught

    @action(detail=False, pagination_class=LimitOffsetPagination)  # caught
    def recent(self, request):
        return Response([])

    @action(detail=False, pagination_class=None)
    def summary(self, request):
        return Response({})


class PagedProducts(generics.ListAPIView):
    # The client cannot change PageNumberPagination's page size: no page_size_query_param.
    queryset = Product.objects.order_by("id")
    pagination_class = PageNumberPagination


class ProductFeed(generics.ListAPIView):
    queryset = Product.objects.order_by("id")
    pagination_class = CursorPagination


class UnpagedOrders(generics.ListAPIView):
    queryset = Order.objects.order_by("id")
    pagination_class = None


class NinjaStyle:
    # Not DRF's class: django-ninja's LimitOffsetPagination caps its limit itself.
    pagination_class = NinjaLimitOffset


class SwitchingView(generics.ListAPIView):
    queryset = Product.objects.order_by("id")

    def get(self, request, *args, **kwargs):
        if request.query_params.get("raw"):
            self.pagination_class = LimitOffsetPagination  # caught
        return super().get(request, *args, **kwargs)
