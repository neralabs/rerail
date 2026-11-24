from django.contrib.auth.models import User

from .base import Plank

from .models import Author, Bin, Book, Code, Config, Doc, Legacy, Loan, Page, Profile, Rack, Setting, Shelf, Tag, Visit


def reported(request, a, kw):
    Author.objects.get_or_create(name=request.POST["name"])  # caught
    Author.objects.update_or_create(name="x", country="FR", defaults={"email": "e"})  # caught
    Book.objects.get_or_create(title="t", defaults={"year": 1})  # caught
    Book.objects.get_or_create(author_id=a.pk, year=1)  # caught
    Book.objects.get_or_create(title="t", edition=2)  # caught
    Book.objects.using("replica").get_or_create(year=1)  # caught
    Tag.objects.get_or_create(  # caught
        label="x",
        slug="y",
    )
    Shelf.objects.get_or_create(name="n", color="red")  # caught
    Profile.objects.update_or_create(bio="b", create_defaults={"bio": "c"})  # caught


def not_reported(request, a, kw, q):
    Author.objects.get_or_create(email="e", defaults={"name": "n"})
    Author.objects.get_or_create(email="e", name="n")
    Author.objects.get_or_create(pk=1)
    Author.objects.get_or_create(id=1, name="n")
    Profile.objects.get_or_create(author=a)
    Profile.objects.get_or_create(author_id=a.pk)
    Tag.objects.get_or_create(code="c", label="x")
    Shelf.objects.get_or_create(name="n", owner=request.user)
    Shelf.objects.get_or_create(name="n", owner_id=1, color="red")
    Book.objects.get_or_create(author=a, title="t", year=1)
    Book.objects.get_or_create(year=1, edition=2)
    Book.objects.get_or_create(isbn="1", year=2020)
    Book.objects.get_or_create(isbn="1", edition=2)
    Book.objects.get_or_create(title__iexact="t")
    Book.objects.get_or_create(**kw)
    Book.objects.get_or_create(q, title="t")
    Book.objects.filter(author=a).get_or_create(title="t")
    a.book_set.get_or_create(title="t")
    Visit.objects.get_or_create(path="/", day=None)
    User.objects.get_or_create(username="u")
    Code.objects.get_or_create(value="v", kind="k")
    Author.objects.get_or_create(nickname="x")
    Legacy.objects.get_or_create(ref="r")
    Config.objects.get_or_create(data={})
    Config.objects.update_or_create(data={})  # caught
    Loan.objects.get_or_create(book=a, key="k")
    Loan.objects.get_or_create(book_id=1, key="k")
    Loan.objects.get_or_create(key="k")  # caught
    Setting.objects.update_or_create(key="k")
    Setting.objects.get_or_create(key="k")  # caught
    Bin.objects.get_or_create(name="n", owner=request.user)  # caught
    Rack.objects.get_or_create(name="n", owner=request.user)
    Page.objects.get_or_create(name="n")
    Doc.objects.get_or_create(name="n")
    Plank.objects.get_or_create(name="n", owner=request.user)
