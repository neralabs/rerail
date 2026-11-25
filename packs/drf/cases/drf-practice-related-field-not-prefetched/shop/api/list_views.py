from django.db.models import prefetch_related_objects
from rest_framework import generics, viewsets
from rest_framework.response import Response

from shop.api.mixins import (
    ActionSerializerMixin,
    CountedListMixin,
    LiteListMixin,
    ModelBaseViewSet,
    PrefetchFilterMixin,
)
from shop.api import PublicBookSerializer
from shop.api.serializers import (
    BookFullSerializer,
    BookNestedSerializer,
    BookPlainSerializer,
    NoteSerializer,
    ProfileSerializer,
    SearchPreferenceSerializer,
    UserPreferenceSerializer,
)
from shop.models import Book, Note, Profile, Review, SearchPreference, UserPreference
from shop.utils import prefetch_related_objects as warm_cache

RELATED = ["author", "publisher"]
MANY = ("reviews", "tags")


# One finding for the view, naming every relation it misses.
class BookFullList(generics.ListAPIView):
    queryset = Book.objects.all()  # caught
    serializer_class = BookFullSerializer


# list() serialises through get_serializer(): the view's serializer lists.
class BookPagedViewSet(viewsets.ReadOnlyModelViewSet):
    queryset = Book.objects.order_by("title")  # caught
    serializer_class = BookFullSerializer

    def list(self, request, *args, **kwargs):
        page = self.paginate_queryset(self.filter_queryset(self.get_queryset()))
        serializer = self.get_serializer(page, many=True)
        return self.get_paginated_response(serializer.data)


class BookCountedList(CountedListMixin, generics.ListAPIView):
    queryset = Book.objects.all()  # caught
    serializer_class = BookNestedSerializer


# The rows come from `self.model` (an inherited get_queryset), through filter_queryset.
class BookModelViewSet(ModelBaseViewSet):
    model = Book
    serializer_class = BookNestedSerializer

    def get_queryset(self):  # caught
        return self.filter_queryset(super().get_queryset().filter(active=True))


class BookModelPlainViewSet(ModelBaseViewSet):
    model = Book
    serializer_class = BookFullSerializer  # caught


# A filter on a ForeignKey that is not unique: many rows per user.
class NoteList(generics.ListAPIView):
    serializer_class = NoteSerializer

    def get_queryset(self):  # caught
        return Note.objects.filter(user=self.request.user)


# A function named like Django's prefetch_related_objects, from the project: it loads nothing the rule can see.
class BookWarmList(generics.ListAPIView):
    queryset = Book.objects.all()  # caught
    serializer_class = BookNestedSerializer

    def paginate_queryset(self, queryset):
        page = super().paginate_queryset(queryset)
        warm_cache(page)
        return page


# The serializer named through its package's re-export (shop/api/__init__.py).
class BookPublicList(generics.ListAPIView):
    queryset = Book.objects.filter(active=True)  # caught
    serializer_class = PublicBookSerializer


# ---- near misses ----------------------------------------------------------------------------------------------------


# Relations loaded through splats of module-level lists and tuples.
class BookSplatList(generics.ListAPIView):
    queryset = Book.objects.select_related(*RELATED).prefetch_related(*MANY)
    serializer_class = BookFullSerializer


# list() serialises another class, directly or in a run base; or no serializer at all.
class BookLiteList(generics.ListAPIView):
    queryset = Book.objects.all()
    serializer_class = BookFullSerializer

    def list(self, request, *args, **kwargs):
        return Response(BookPlainSerializer(self.get_queryset(), many=True).data)


class BookLiteMixinList(LiteListMixin, generics.ListAPIView):
    queryset = Book.objects.all()
    serializer_class = BookFullSerializer


class BookIdsViewSet(viewsets.ModelViewSet):
    queryset = Book.objects.all()
    serializer_class = BookFullSerializer

    def list(self, request, *args, **kwargs):
        return Response(list(self.get_queryset().values_list("id", flat=True)))


class BookActionViewSet(ActionSerializerMixin, viewsets.ModelViewSet):
    queryset = Book.objects.all()
    serializer_class = BookFullSerializer


# prefetch_related_objects() loads the page: in paginate_queryset, in list(), in a base's filter_queryset.
class BookPrefetchPageList(generics.ListAPIView):
    queryset = Book.objects.all()
    serializer_class = BookFullSerializer

    def paginate_queryset(self, queryset):
        page = super().paginate_queryset(queryset)
        prefetch_related_objects(page, "author", "publisher", "reviews", "tags")
        return page


class BookPrefetchListViewSet(viewsets.ReadOnlyModelViewSet):
    queryset = Book.objects.all()
    serializer_class = BookNestedSerializer

    def list(self, request, *args, **kwargs):
        rows = list(self.filter_queryset(self.get_queryset()))
        prefetch_related_objects(rows, "author")
        return Response(self.get_serializer(rows, many=True).data)


class BookPrefetchFilterList(PrefetchFilterMixin, generics.ListAPIView):
    queryset = Book.objects.all()
    serializer_class = BookNestedSerializer


# Arguments the rule cannot read.
class BookDynamicList(generics.ListAPIView):
    serializer_class = BookFullSerializer
    related = ("author",)

    def get_queryset(self):
        return Book.objects.select_related(*self.related)


class BookLookupList(generics.ListAPIView):
    serializer_class = BookNestedSerializer

    def get_queryset(self):
        lookup = self.request.query_params.get("expand", "author")
        return Book.objects.select_related(lookup)


# One row per user: a OneToOneField, a third-party OneToOneField (primary key), a unique ForeignKey by its column.
class UserPreferenceViewSet(viewsets.ModelViewSet):
    queryset = UserPreference.objects
    serializer_class = UserPreferenceSerializer

    def get_queryset(self):
        return self.queryset.filter(user=self.request.user)


class SearchPreferenceViewSet(viewsets.ModelViewSet):
    queryset = SearchPreference.objects
    serializer_class = SearchPreferenceSerializer

    def get_queryset(self):
        return self.queryset.filter(user=self.request.user)


class ProfileList(generics.ListAPIView):
    serializer_class = ProfileSerializer

    def get_queryset(self):
        return Profile.objects.filter(owner_id=self.request.user.id)


# `self.model` names another model than the serializer's, or none.
class ReviewModelViewSet(ModelBaseViewSet):
    model = Review
    serializer_class = BookNestedSerializer


class UnsetModelViewSet(ModelBaseViewSet):
    serializer_class = BookNestedSerializer
