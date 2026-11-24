from rest_framework import generics

from shop.models import Book


class BookBaseView(generics.GenericAPIView):
    queryset = Book.objects.all()
