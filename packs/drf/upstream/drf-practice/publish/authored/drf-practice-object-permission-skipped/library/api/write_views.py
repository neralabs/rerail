from rest_framework import generics, viewsets
from rest_framework.decorators import action, api_view, permission_classes
from rest_framework.permissions import IsAuthenticated
from rest_framework.response import Response

from library.api.permissions import IsOwner
from library.api.serializers import BookSerializer
from library.models import Book, Shelf


# Rows chosen by the URL and deleted or updated in one query: nothing fetches them, so nothing checks them.
class BookDelete(generics.GenericAPIView):
    permission_classes = [IsOwner]
    serializer_class = BookSerializer

    def delete(self, request, pk):
        Book.objects.filter(pk=pk).delete()  # caught
        return Response(status=204)


# The Tandoor shape: a mixin's detail actions read the view's own queryset by the URL's pk.
class RelationMixin:
    @action(detail=True, methods=["get"])
    def protecting(self, request, pk=None):
        obj = self.queryset.filter(pk=pk, organization=request.data.get("org")).first()  # caught
        return Response({"id": obj.pk})


class BookWriteViewSet(RelationMixin, viewsets.ModelViewSet):
    permission_classes = [IsAuthenticated, IsOwner]
    queryset = Book.objects.all()
    serializer_class = BookSerializer

    def partial_update(self, request, pk=None):
        Book.objects.filter(pk=pk).update(title=request.data["title"])  # caught
        return Response(status=204)

    def update(self, request, *args, **kwargs):
        self.get_queryset().filter(id=kwargs["pk"]).update(title=request.data["title"])  # caught
        return Response(status=204)

    def destroy(self, request, *args, **kwargs):
        self.queryset.filter(pk=self.kwargs["pk"]).delete()  # caught
        return Response(status=204)

    def retrieve(self, request, pk=None):
        book = Book.objects.filter(pk=pk)[0]  # caught
        return Response(BookSerializer(book).data)

    @action(detail=True, methods=["post"])
    def retitle(self, request, pk=None):
        books = Book.objects
        books.filter(pk=pk).update(title=request.data["title"])  # caught
        return Response(status=204)

    @action(detail=True, methods=["post"])
    def archive(self, request, pk=None):
        book_id = self.kwargs["pk"]
        Book.live.filter(id=book_id).update(archived=True)  # caught
        return Response(status=204)

    @action(detail=True, methods=["post"])
    def stamp(self, request, pk=None):
        Book.objects.filter(pk=pk).update(editor=request.user)  # caught
        return Response(status=204)

    @action(detail=True, methods=["get"])
    def newest(self, request, pk=None):
        book = Book.objects.filter(shelf_id=pk).order_by("id").last()  # caught
        return Response(BookSerializer(book).data)

    @action(detail=True, methods=["get"])
    def latest_copy(self, request, pk=None):
        book = Book.live.filter(title=pk).latest("id")  # caught
        return Response(BookSerializer(book).data)

    @action(detail=True, methods=["get"])
    def only_one(self, request, pk=None):
        rows = self.get_queryset()
        book = rows.filter(pk=pk).get()  # caught
        return Response(BookSerializer(book).data)

    # The lookup or the queryset is limited to the requester.
    @action(detail=True, methods=["post"])
    def drop_mine(self, request, pk=None):
        Book.objects.filter(pk=pk, owner=request.user).delete()
        return Response(status=204)

    # The row was checked by get_object() first.
    @action(detail=True, methods=["post"])
    def touch(self, request, pk=None):
        book = self.get_object()
        Book.objects.filter(pk=book.pk).update(title=book.title)
        return Response(status=204)

    # Not chosen by the URL, or another model's rows.
    @action(detail=True, methods=["post"])
    def reset_drafts(self, request, pk=None):
        Book.objects.filter(title="draft").update(title="")
        return Response(status=204)

    @action(detail=True, methods=["post"])
    def clear_shelf(self, request, pk=None):
        Shelf.objects.filter(pk=pk).update(slug="")
        return Response(status=204)

    # A list action: many rows by design.
    @action(detail=False, methods=["post"])
    def bulk_delete(self, request):
        Book.objects.filter(pk__in=request.data["ids"]).delete()
        return Response(status=204)

    # A read that stops at the queryset, and a count.
    @action(detail=True, methods=["get"])
    def exists(self, request, pk=None):
        return Response({"exists": Book.objects.filter(pk=pk).exists()})


# Permissions from get_permissions(): one return of a list of instances.
class BookPermsView(generics.GenericAPIView):
    serializer_class = BookSerializer

    def get_permissions(self):
        return [IsAuthenticated(), IsOwner()]

    def put(self, request, pk):
        book = Book.objects.get(pk=pk)  # caught
        return Response(BookSerializer(book).data)


# get_permissions() replaces permission_classes: IsOwner never runs here.
class BookPermsOverride(generics.GenericAPIView):
    serializer_class = BookSerializer
    permission_classes = [IsOwner]

    def get_permissions(self):
        return [IsAuthenticated()]

    def put(self, request, pk):
        book = Book.objects.get(pk=pk)
        return Response(BookSerializer(book).data)


@api_view(["DELETE"])
@permission_classes([IsAuthenticated, IsOwner])
def drop_book(request, pk):
    Book.objects.filter(pk=pk).delete()  # caught
    return Response(status=204)


@api_view(["DELETE"])
@permission_classes([IsAuthenticated, IsOwner])
def drop_my_book(request, pk):
    Book.objects.filter(pk=pk, owner=request.user).delete()
    return Response(status=204)
