from django.shortcuts import get_object_or_404
from django.http import HttpResponseForbidden
from django.views import View
from rest_framework import generics, permissions, viewsets
from rest_framework.decorators import action
from rest_framework.permissions import IsAuthenticated
from rest_framework.response import Response
from rest_framework.settings import api_settings

from library.api.access import has_perms_owner_aware
from library.api.base import OwnerAPIView, SignedInAPIView
from library.api.permissions import IsMember, IsOwner, IsStaff, IsStrictOwner
from library.api.serializers import BookSerializer, ShelfSerializer
from library.models import Book, Shelf


# The object permission comes from a base view in another module; the view's rows are Book (its serializer).
class BookDetail(OwnerAPIView):
    serializer_class = BookSerializer

    def get(self, request, pk):
        book = get_object_or_404(Book, pk=pk)  # caught
        return Response(BookSerializer(book).data)

    def delete(self, request, pk):
        Book.objects.get(pk=pk).delete()  # caught
        return Response(status=204)

    def put(self, request, pk):
        book = self.get_object()
        serializer = BookSerializer(book, data=request.data)
        serializer.is_valid(raise_exception=True)
        serializer.save()
        return Response(serializer.data)

    def patch(self, request, pk):
        book = get_object_or_404(Book, pk=pk)
        self.check_object_permissions(request, book)
        return Response(BookSerializer(book).data)


# No permission_classes: the settings default (HasObjectPermission, named by a dotted string) applies.
class BookViewSet(viewsets.ModelViewSet):
    queryset = Book.objects.all()
    serializer_class = BookSerializer

    def retrieve(self, request, pk=None):
        book = Book.objects.filter(pk=pk).first()  # caught
        return Response(BookSerializer(book).data)

    def update(self, request, pk=None):
        book = self.get_queryset().get(pk=pk)  # caught
        return Response(BookSerializer(book).data)

    @action(detail=True, methods=["post"])
    def archive(self, request, pk=None):
        book = get_object_or_404(self.queryset, pk=pk)  # caught
        book.delete()
        return Response(status=204)

    @action(detail=True, methods=["post"])
    def move(self, request, pk=None):
        book = get_object_or_404(Book.objects.select_related("shelf"), pk=pk)  # caught
        shelf = Shelf.objects.get(slug=request.data["shelf"])
        book.shelf = shelf
        book.save()
        return Response(status=204)

    def partial_update(self, request, pk=None):
        book = self._load(pk)
        return Response(BookSerializer(book).data)

    def _load(self, pk):
        book = Book.objects.get(pk=pk)
        self._authorise(book)
        return book

    def _authorise(self, book):
        self.check_object_permissions(self.request, book)

    def destroy(self, request, pk=None):
        book = Book.objects.get(pk=pk, owner=request.user)
        book.delete()
        return Response(status=204)

    @action(detail=True, methods=["get"])
    def same_org(self, request, pk=None):
        book = Book.objects.filter(pk=pk, organization=request.user.active_organization).first()
        return Response(BookSerializer(book).data)

    @action(detail=True, methods=["get"])
    def shelf(self, request, pk=None):
        shelf = get_object_or_404(Shelf, pk=request.query_params["shelf"])
        return Response(ShelfSerializer(shelf).data)

    @action(detail=True, methods=["get"])
    def labels(self, request, pk=None):
        user = request.user
        book = Book.objects.get(pk=pk)
        if not has_perms_owner_aware(user, "view_book", book):
            return HttpResponseForbidden("Insufficient permissions")
        return Response(BookSerializer(book).data)

    @action(detail=True, methods=["get"])
    def mine(self, request, pk=None):
        user = request.user
        book = Book.objects.get(pk=pk, owner=user)
        return Response(BookSerializer(book).data)

    def list(self, request):
        first = Book.objects.filter(title=request.query_params.get("title")).first()
        return Response(BookSerializer(first).data)

    def create(self, request):
        shelf = Shelf.objects.get(pk=request.data["shelf"])
        return Response(ShelfSerializer(shelf).data)

    @action(detail=False, methods=["get"])
    def latest(self, request):
        book = Book.objects.filter(title="x").first()
        return Response(BookSerializer(book).data)

    # The action's own permission_classes replace the default: nothing checks objects here.
    @action(detail=True, methods=["get"], permission_classes=[IsAuthenticated])
    def preview(self, request, pk=None):
        book = Book.objects.get(pk=pk)
        return Response(BookSerializer(book).data)


# The view itself checks no objects, but one action names an object permission.
class ShelfViewSet(viewsets.ReadOnlyModelViewSet):
    queryset = Shelf.objects.all()
    serializer_class = ShelfSerializer
    permission_classes = [IsAuthenticated]

    @action(detail=True, methods=["post"], permission_classes=[IsAuthenticated, IsStrictOwner])
    def rename(self, request, pk=None):
        shelf = Shelf.objects.get(pk=pk)  # caught
        shelf.slug = request.data["slug"]
        shelf.save()
        return Response(status=204)

    @action(detail=True, methods=["get"])
    def books(self, request, pk=None):
        shelf = Shelf.objects.get(pk=pk)
        return Response(ShelfSerializer(shelf).data)

    def retrieve(self, request, pk=None):
        shelf = Shelf.objects.get(pk=pk)
        return Response(ShelfSerializer(shelf).data)


# DjangoObjectPermissions, through a `+` of the default and a list.
class GuardedBook(generics.RetrieveDestroyAPIView):
    queryset = Book.objects.all()
    serializer_class = BookSerializer
    permission_classes = api_settings.DEFAULT_PERMISSION_CLASSES + [permissions.DjangoObjectPermissions]

    def destroy(self, request, *args, **kwargs):
        book = get_object_or_404(Book, pk=kwargs["pk"])  # caught
        book.delete()
        return Response(status=204)

    def retrieve(self, request, *args, **kwargs):
        Book.objects.get(pk=kwargs["pk"])
        return super().retrieve(request, *args, **kwargs)


# An APIView with no model: a fetch counts when its lookup reads a URL kwarg.
class ShelfBySlug(OwnerAPIView):
    def get(self, request, slug):
        shelf = Shelf.objects.get(slug=slug)  # caught
        return Response(ShelfSerializer(shelf).data)

    def put(self, request, slug):
        shelf = Shelf.objects.get(slug=request.data["slug"])
        return Response(ShelfSerializer(shelf).data)


# The queryset is scoped to the requester: the fetch reads only their rows.
class MyBooks(viewsets.ReadOnlyModelViewSet):
    serializer_class = BookSerializer
    permission_classes = [IsAuthenticated, IsOwner]

    def get_queryset(self):
        return Book.objects.filter(owner=self.request.user)

    def retrieve(self, request, pk=None):
        book = self.get_queryset().get(pk=pk)
        return Response(BookSerializer(book).data)


# No object permission anywhere: nothing is skipped.
class StaffBook(SignedInAPIView):
    serializer_class = BookSerializer

    def get(self, request, pk):
        book = get_object_or_404(Book, pk=pk)
        return Response(BookSerializer(book).data)


class StaffShelf(generics.RetrieveAPIView):
    serializer_class = ShelfSerializer
    permission_classes = [IsStaff]

    def get(self, request, pk):
        shelf = Shelf.objects.get(pk=pk)
        return Response(ShelfSerializer(shelf).data)


# The view authorises the row itself with the object permission class.
class CheckedBook(OwnerAPIView):
    serializer_class = BookSerializer

    def get(self, request, pk):
        book = Book.objects.get(pk=pk)
        if not IsOwner().has_object_permission(request, self, book):
            return Response(status=403)
        return Response(BookSerializer(book).data)


# A plain Django view is no DRF view.
class PlainBook(View):
    permission_classes = [IsOwner]

    def get(self, request, pk):
        book = get_object_or_404(Book, pk=pk)
        return Response(BookSerializer(book).data)


class MemberBook(generics.RetrieveAPIView):
    queryset = Book.objects.all()
    serializer_class = BookSerializer
    permission_classes = [IsAuthenticated, IsMember]

    def get(self, request, pk):
        book = Book.objects.get(pk=pk)
        return Response(BookSerializer(book).data)


# A base viewset restricts self.queryset to the requester on every request (NetBox's initial()).
class RestrictedViewSet(viewsets.ModelViewSet):
    permission_classes = [IsAuthenticated, IsOwner]

    def initial(self, request, *args, **kwargs):
        super().initial(request, *args, **kwargs)
        self.queryset = self.queryset.filter(owner=request.user)


class RestrictedBooks(RestrictedViewSet):
    queryset = Book.objects.all()
    serializer_class = BookSerializer

    @action(detail=True, methods=["get"])
    def cover(self, request, pk=None):
        book = get_object_or_404(self.queryset, pk=pk)
        return Response(BookSerializer(book).data)
