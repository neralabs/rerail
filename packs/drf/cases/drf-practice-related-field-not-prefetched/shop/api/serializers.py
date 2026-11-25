from rest_framework import serializers

from shop.models import Article, Author, Book, Note, Profile, Review, SearchPreference, Theme, UserPreference


class AuthorSerializer(serializers.ModelSerializer):
    class Meta:
        model = Author
        fields = ["id", "name"]


class ReviewSerializer(serializers.ModelSerializer):
    class Meta:
        model = Review
        fields = ["id", "text"]


# (a) a nested serializer on a ForeignKey.
class BookNestedSerializer(serializers.ModelSerializer):
    author = AuthorSerializer(read_only=True)

    class Meta:
        model = Book
        fields = ["id", "title", "author"]


# (b) a dotted source through a ForeignKey.
class BookPublisherSerializer(serializers.ModelSerializer):
    publisher_name = serializers.CharField(source="publisher.name", read_only=True)

    class Meta:
        model = Book
        fields = ["id", "title", "publisher_name"]


# (c) a ManyToManyField named in Meta.fields; the plain ForeignKey `author` beside it reads author_id only.
class BookTagsSerializer(serializers.ModelSerializer):
    class Meta:
        model = Book
        fields = ["id", "title", "author", "tags"]


# (d) depth = 1 nests every relation in fields.
class BookDepthSerializer(serializers.ModelSerializer):
    class Meta:
        model = Book
        fields = ["id", "publisher"]
        depth = 1


# A reverse ForeignKey, nested by its related_name; a reverse one by its default `<model>_set` name.
class BookReviewsSerializer(serializers.ModelSerializer):
    reviews = ReviewSerializer(many=True, read_only=True)

    class Meta:
        model = Book
        fields = ["id", "reviews"]


class BookChaptersSerializer(serializers.ModelSerializer):
    chapters = serializers.PrimaryKeyRelatedField(source="chapter_set", many=True, read_only=True)

    class Meta:
        model = Book
        fields = ["id", "chapters"]


# A related field that renders the related object, declared under the relation's own name.
class BookAuthorNameSerializer(serializers.ModelSerializer):
    author = serializers.StringRelatedField()

    class Meta:
        model = Book
        fields = ["id", "author"]


# "__all__" covers the ManyToManyField too.
class BookAllSerializer(serializers.ModelSerializer):
    class Meta:
        model = Book
        fields = "__all__"


# A subclass inherits the nested field and Meta of its base.
class BookNestedChildSerializer(BookNestedSerializer):
    pass


# ---- near misses ----------------------------------------------------------------------------------------------------


# Plain ForeignKeys: PrimaryKeyRelatedField reads `author_id`, no query.
class BookPlainSerializer(serializers.ModelSerializer):
    author_id_too = serializers.PrimaryKeyRelatedField(source="author", read_only=True)

    class Meta:
        model = Book
        fields = ["id", "title", "author", "publisher", "author_id_too"]


# write_only fields read nothing. A SerializerMethodField's body is not read (a gap: get_tag_count queries per row).
class BookWriteSerializer(serializers.ModelSerializer):
    author = AuthorSerializer(write_only=True)
    tag_count = serializers.SerializerMethodField()

    class Meta:
        model = Book
        fields = ["id", "author", "tags", "tag_count"]
        extra_kwargs = {"tags": {"write_only": True}}

    def get_tag_count(self, obj):
        return obj.tags.count()


# to_representation is overridden: it may never read the field.
class BookCustomSerializer(serializers.ModelSerializer):
    author = AuthorSerializer(read_only=True)

    class Meta:
        model = Book
        fields = ["id", "author"]

    def to_representation(self, instance):
        return {"id": instance.pk}


# A dotted source on a plain field, not a relation.
class BookTitleSerializer(serializers.ModelSerializer):
    title_upper = serializers.CharField(source="title.upper", read_only=True)

    class Meta:
        model = Book
        fields = ["id", "title_upper"]


# The model's default manager loads the relation itself (the rule cannot see what it loads).
class ArticleSerializer(serializers.ModelSerializer):
    author = AuthorSerializer(read_only=True)

    class Meta:
        model = Article
        fields = ["id", "author"]


# Reads two one-row relations and two many ones: one finding lists all four.
class BookFullSerializer(serializers.ModelSerializer):
    author = AuthorSerializer(read_only=True)
    publisher_name = serializers.CharField(source="publisher.name", read_only=True)
    reviews = ReviewSerializer(many=True, read_only=True)

    class Meta:
        model = Book
        fields = ["id", "author", "publisher_name", "reviews", "tags"]


class ThemeSerializer(serializers.ModelSerializer):
    class Meta:
        model = Theme
        fields = ["id", "name"]


class UserPreferenceSerializer(serializers.ModelSerializer):
    theme = ThemeSerializer(read_only=True)

    class Meta:
        model = UserPreference
        fields = ["user", "theme", "shared_with"]


class SearchPreferenceSerializer(serializers.ModelSerializer):
    theme = ThemeSerializer(read_only=True)

    class Meta:
        model = SearchPreference
        fields = ["user", "theme"]


class ProfileSerializer(serializers.ModelSerializer):
    theme = ThemeSerializer(read_only=True)

    class Meta:
        model = Profile
        fields = ["id", "theme"]


class NoteSerializer(serializers.ModelSerializer):
    theme = ThemeSerializer(read_only=True)

    class Meta:
        model = Note
        fields = ["id", "theme"]
