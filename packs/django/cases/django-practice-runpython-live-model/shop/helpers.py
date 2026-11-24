from django.contrib.auth.models import User

from shop.models import Author, Book, Shelf


def recount_books():
    for author in Author.objects.all():
        author.book_count = Book.objects.filter(author=author).count()
        author.save()


def make_default_shelf():
    Shelf(name="default").save()


def count_via_default_manager():
    return Book._default_manager.count()


def refresh_counts():
    _recount_later()


def _recount_later():
    recount_books()


# The model arrives as an argument: inside, `book_model` is whatever the caller passed (a historical model).
def fill_slugs(book_model):
    for book in book_model.objects.filter(slug=""):
        book.slug = book.title.lower()
        book.save()


# A parameter that shadows the class name is still a parameter.
def count_books(Book):
    return Book.objects.count()


# The class only for isinstance, a constant and its exception: no query.
def is_done(obj):
    if isinstance(obj, Book) and obj.status == Book.STATUS_DONE:
        return True
    try:
        return obj.author is not None
    except Book.DoesNotExist:
        return False


# Django's own model: not a model class of the run.
def make_admin():
    User.objects.create(username="admin")


# The query runs only when the caller leaves `per_day` out.
def due_in(start, per_day=None):
    if per_day is None:
        per_day = Shelf.objects.count()
    return start + per_day


def due_in_one(start):
    return due_in(start, per_day=1)


def shelf_count():
    return Shelf.objects.count()


# The call that queries runs only when the caller leaves `per_day` out.
def due_in_days(start, per_day=None):
    if per_day is None:
        per_day = shelf_count()
    return start + per_day


def due_in_two(start):
    return due_in_days(start, per_day=2)


def make_named_shelf(name):
    shelf = Shelf(name=name)
    shelf.save()
    return shelf


# An unsaved instance, built only to use its methods: no query.
def shelf_label(name):
    return str(Shelf(name=name))
