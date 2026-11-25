from ninja.pagination import LimitOffsetPagination as NinjaLimitOffset
from rest_framework import generics, pagination, viewsets
from rest_framework import pagination as pg
from rest_framework.decorators import action
from rest_framework.decorators import api_view
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


# The built-in class instantiated in a function view: the same open ?limit=.
@api_view(["GET"])
def product_rows(request):
    paginator = LimitOffsetPagination()  # caught
    page = paginator.paginate_queryset(Product.objects.order_by("id"), request)
    return paginator.get_paginated_response([p.pk for p in page])


@api_view(["GET"])
def order_rows(request):
    paginator = pg.LimitOffsetPagination()  # caught
    page = paginator.paginate_queryset(Order.objects.order_by("id"), request)
    return paginator.get_paginated_response([o.pk for o in page])


@api_view(["GET"])
def ninja_rows(request):
    # django-ninja's class caps its own limit.
    paginator = NinjaLimitOffset()
    return Response(paginator)


@api_view(["GET"])
def paged_rows(request):
    paginator = PageNumberPagination()
    page = paginator.paginate_queryset(Order.objects.order_by("id"), request)
    return paginator.get_paginated_response([o.pk for o in page])
