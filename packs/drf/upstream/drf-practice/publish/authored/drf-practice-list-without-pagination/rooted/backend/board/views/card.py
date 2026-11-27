from rest_framework import serializers, viewsets

from board.models import Card
from board.views.base import CardListBase


class CardSerializer(serializers.ModelSerializer):
    class Meta:
        model = Card
        fields = ["id", "title"]


# Registered with a router (which routes list) under an alias.
class CardViewSet(viewsets.ReadOnlyModelViewSet):  # caught
    queryset = Card.objects.all()
    serializer_class = CardSerializer


class PinnedCardList(CardListBase):  # caught
    serializer_class = CardSerializer
