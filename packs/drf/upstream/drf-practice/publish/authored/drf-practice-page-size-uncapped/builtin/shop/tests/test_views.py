from rest_framework import generics
from rest_framework.pagination import LimitOffsetPagination

from shop.models import Product


class FixtureList(generics.ListAPIView):
    queryset = Product.objects.order_by("id")
    pagination_class = LimitOffsetPagination
