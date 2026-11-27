from django.shortcuts import get_object_or_404
from rest_framework.response import Response

from library.api.base import OwnerAPIView
from library.models import Book


class FakeDetail(OwnerAPIView):
    def get(self, request, pk):
        book = get_object_or_404(Book, pk=pk)
        return Response({"id": book.pk})
