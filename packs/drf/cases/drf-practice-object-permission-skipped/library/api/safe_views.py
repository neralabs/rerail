from django.shortcuts import get_object_or_404
from rest_framework import generics, permissions, viewsets
from rest_framework.decorators import action, api_view, permission_classes
from rest_framework.permissions import IsAuthenticated
from rest_framework.response import Response

from library.api.permissions import (
    ConstReadOrOwner,
    IsOwner,
    IsOwnerOrReadOnly,
    LiveOwnerOrReadOnly,
    OwnerOrReadOnly,
    PublicOrOwner,
    ReadOrOwner,
    ShelfOwnerOrReadOnly,
    ViewObjectPermissions,
)
from library.api.serializers import BookSerializer, ShelfSerializer
from library.models import Book, Shelf


# The object check returns True for every safe request: a read handler skips nothing; a write handler does.
class BookRead(generics.GenericAPIView):
    permission_classes = [IsOwnerOrReadOnly]
    serializer_class = BookSerializer

    def get(self, request, pk):
        book = get_object_or_404(Book, pk=pk)
        return Response(BookSerializer(book).data)

    def put(self, request, pk):
        book = Book.objects.get(pk=pk)  # caught
        return Response(BookSerializer(book).data)


class BookReadViewSet(viewsets.ModelViewSet):
    queryset = Book.objects.all()
    serializer_class = BookSerializer
    permission_classes = [IsAuthenticated, ReadOrOwner]

    def retrieve(self, request, pk=None):
        book = Book.objects.filter(pk=pk).first()
        return Response(BookSerializer(book).data)

    @action(detail=True)
    def cover(self, request, pk=None):
        book = Book.objects.get(pk=pk)
        return Response(BookSerializer(book).data)

    @action(detail=True, methods=["get", "head"])
    def spine(self, request, pk=None):
        book = Book.objects.get(pk=pk)
        return Response(BookSerializer(book).data)

    @action(detail=True, methods=["get", "post"])
    def borrow(self, request, pk=None):
        book = Book.objects.get(pk=pk)  # caught
        return Response(BookSerializer(book).data)

    def destroy(self, request, pk=None):
        self.get_queryset().filter(pk=pk).delete()  # caught
        return Response(status=204)


class ShelfRead(generics.RetrieveAPIView):
    queryset = Shelf.objects.all()
    serializer_class = ShelfSerializer
    permission_classes = [ConstReadOrOwner]

    def get(self, request, pk):
        shelf = Shelf.objects.get(pk=pk)
        return Response(ShelfSerializer(shelf).data)


class ShelfInherited(generics.RetrieveAPIView):
    queryset = Shelf.objects.all()
    serializer_class = ShelfSerializer
    permission_classes = [ShelfOwnerOrReadOnly]

    def get(self, request, pk):
        shelf = Shelf.objects.get(pk=pk)
        return Response(ShelfSerializer(shelf).data)


# DjangoObjectPermissions requires no permission for GET, HEAD and OPTIONS.
class ShelfModelPerms(generics.RetrieveAPIView):
    queryset = Shelf.objects.all()
    serializer_class = ShelfSerializer
    permission_classes = [permissions.DjangoObjectPermissions]

    def get(self, request, pk):
        shelf = Shelf.objects.get(pk=pk)
        return Response(ShelfSerializer(shelf).data)


class ShelfViewPerms(generics.RetrieveAPIView):
    queryset = Shelf.objects.all()
    serializer_class = ShelfSerializer
    permission_classes = [ViewObjectPermissions]

    def get(self, request, pk):
        shelf = Shelf.objects.get(pk=pk)  # caught
        return Response(ShelfSerializer(shelf).data)


# One object permission open to reads, one not: the read skips IsOwner.
class BookMixedRead(generics.GenericAPIView):
    permission_classes = [IsAuthenticated, IsOwnerOrReadOnly, IsOwner]
    serializer_class = BookSerializer

    def get(self, request, pk):
        book = Book.objects.get(pk=pk)  # caught
        return Response(BookSerializer(book).data)


# Read handlers under permissions whose safe-request answer is not a plain True.
class BookNamedReadOnly(generics.GenericAPIView):
    permission_classes = [OwnerOrReadOnly]
    serializer_class = BookSerializer

    def get(self, request, pk):
        book = Book.objects.get(pk=pk)  # caught
        return Response(BookSerializer(book).data)


class BookPublic(generics.GenericAPIView):
    permission_classes = [PublicOrOwner]
    serializer_class = BookSerializer

    def get(self, request, pk):
        book = Book.objects.get(pk=pk)  # caught
        return Response(BookSerializer(book).data)


class BookLive(generics.GenericAPIView):
    permission_classes = [LiveOwnerOrReadOnly]
    serializer_class = BookSerializer

    def get(self, request, pk):
        book = Book.objects.get(pk=pk)  # caught
        return Response(BookSerializer(book).data)


@api_view(["GET"])
@permission_classes([IsOwnerOrReadOnly])
def read_book(request, pk):
    book = get_object_or_404(Book, pk=pk)
    return Response(BookSerializer(book).data)


@api_view
@permission_classes([ReadOrOwner])
def peek_book(request, pk):
    book = Book.objects.get(pk=pk)
    return Response(BookSerializer(book).data)


@api_view(["GET", "DELETE"])
@permission_classes([IsOwnerOrReadOnly])
def read_or_drop_book(request, pk):
    book = get_object_or_404(Book, pk=pk)  # caught
    if request.method == "DELETE":
        book.delete()
    return Response(BookSerializer(book).data)
