from rest_framework.viewsets import ModelViewSet

from shop.tasks import notify_order


class OrderViewSet(ModelViewSet):
    def perform_create(self, serializer):
        order = serializer.save()
        notify_order.delay(order.pk)  # caught
