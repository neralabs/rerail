from rest_framework import generics

from shop.api.serializers import BookNestedSerializer
from shop.models import Book


class FakeBookList(generics.ListAPIView):
    queryset = Book.objects.all()
    serializer_class = BookNestedSerializer
