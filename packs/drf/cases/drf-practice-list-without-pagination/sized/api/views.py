from rest_framework import generics, serializers
from rest_framework.pagination import CursorPagination, LimitOffsetPagination

from api.models import Event


class EventSerializer(serializers.ModelSerializer):
    class Meta:
        model = Event
        fields = ["id", "name"]


class EventList(generics.ListAPIView):
    queryset = Event.objects.all()
    serializer_class = EventSerializer


class EventOffsets(generics.ListAPIView):
    queryset = Event.objects.all()
    serializer_class = EventSerializer
    pagination_class = LimitOffsetPagination


class EventFeed(CursorPagination):
    ordering = "-id"


class EventCursor(generics.ListAPIView):
    queryset = Event.objects.all()
    serializer_class = EventSerializer
    pagination_class = EventFeed
