from django.shortcuts import get_object_or_404
from rest_framework.decorators import api_view, permission_classes
from rest_framework.permissions import AllowAny, IsAuthenticated
from rest_framework.response import Response

from library.api.permissions import IsOwner
from library.api.serializers import BookSerializer
from library.models import Book


@api_view(["GET", "DELETE"])
@permission_classes([IsAuthenticated, IsOwner])
def book_detail(request, pk):
    book = get_object_or_404(Book, pk=pk)  # caught
    if request.method == "DELETE":
        book.delete()
    return Response(BookSerializer(book).data)


# No @permission_classes: the settings default (HasObjectPermission) applies.
@api_view(["GET"])
def book_by_id(request, book_id):
    book = Book.objects.get(id=book_id)  # caught
    return Response(BookSerializer(book).data)


@api_view(["GET"])
@permission_classes([IsAuthenticated, IsOwner])
def checked_book(request, pk):
    book = get_object_or_404(Book, pk=pk)
    if not IsOwner().has_object_permission(request, None, book):
        return Response(status=403)
    return Response(BookSerializer(book).data)


@api_view(["GET"])
@permission_classes([IsAuthenticated, IsOwner])
def my_book(request, pk):
    book = get_object_or_404(Book, pk=pk, owner=request.user)
    return Response(BookSerializer(book).data)


@api_view(["GET"])
@permission_classes([AllowAny])
def public_book(request, pk):
    book = get_object_or_404(Book, pk=pk)
    return Response(BookSerializer(book).data)


@api_view(["GET"])
@permission_classes([IsAuthenticated, IsOwner])
def search(request):
    book = Book.objects.filter(title=request.query_params["q"]).first()
    return Response(BookSerializer(book).data)


# Not a view: no decorator.
def load_book(pk):
    return Book.objects.get(pk=pk)
