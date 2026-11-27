from rest_framework import generics, pagination, serializers

from api.models import Event
from api.pagination import ClientSizedPages, ComputedPages, ConfiguredPages, EventFeed, SizedPages


class EventSerializer(serializers.ModelSerializer):
    class Meta:
        model = Event
        fields = ["id", "name"]


# Covered by the default (reported once, at the setting), not here.
class EventList(generics.ListAPIView):
    queryset = Event.objects.all()
    serializer_class = EventSerializer


class EventPages(generics.ListAPIView):
    queryset = Event.objects.all()
    serializer_class = EventSerializer
    pagination_class = pagination.PageNumberPagination  # caught


class EventClientSized(generics.ListAPIView):
    queryset = Event.objects.all()
    serializer_class = EventSerializer
    pagination_class = ClientSizedPages  # caught


class EventSized(generics.ListAPIView):
    queryset = Event.objects.all()
    serializer_class = EventSerializer
    pagination_class = SizedPages


class EventCursor(generics.ListAPIView):
    queryset = Event.objects.all()
    serializer_class = EventSerializer
    pagination_class = EventFeed


class EventConfigured(generics.ListAPIView):
    queryset = Event.objects.all()
    serializer_class = EventSerializer
    pagination_class = ConfiguredPages


class EventComputed(generics.ListAPIView):
    queryset = Event.objects.all()
    serializer_class = EventSerializer
    pagination_class = ComputedPages


class EventAll(generics.ListAPIView):
    queryset = Event.objects.all()
    serializer_class = EventSerializer
    pagination_class = None
