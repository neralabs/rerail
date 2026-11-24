from rest_framework import viewsets

from shop.api.serializers import OrderSerializer
from shop.models import Order


class FakeOrderViewSet(viewsets.ModelViewSet):
    queryset = Order.objects.all()
    serializer_class = OrderSerializer
