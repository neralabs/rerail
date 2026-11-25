from django.db import models
from rest_framework import viewsets
from rest_framework.response import Response

from shop.api.serializers import BookPlainSerializer, BookTitleSerializer


# A base whose list() serialises with a lighter class: its subclasses' serializer_class never lists.
class LiteListMixin:
    def list(self, request, *args, **kwargs):
        return Response(BookTitleSerializer(self.get_queryset(), many=True).data)


# A base whose list() calls super().list(): the view's serializer still lists.
class CountedListMixin:
    def list(self, request, *args, **kwargs):
        response = super().list(request, *args, **kwargs)
        response["X-Count"] = str(len(response.data))
        return response


# A base whose get_serializer_class() picks another class for the list action.
class ActionSerializerMixin:
    def get_serializer_class(self):
        if self.action == "list":
            return BookPlainSerializer
        return super().get_serializer_class()


# A base that prefetches the page after filtering, through the `models` module.
class PrefetchFilterMixin:
    def filter_queryset(self, queryset):
        rows = list(super().filter_queryset(queryset))
        models.prefetch_related_objects(rows, "author", "tags")
        return rows


# The rows come from `self.model`, as a project base view sets it up.
class ModelBaseViewSet(viewsets.ModelViewSet):
    model = None

    def get_queryset(self):
        return self.model.objects.all()
