from .models import Archive, Book, Chapter, Counter, Label, Note, Plain, Tag


def edit(request):
    Book.objects.filter(title="x").update(title="y")  # caught
    Book.objects.bulk_create([Book(title="a")])  # caught
    Book.objects.bulk_update([], ["title"])  # caught
    Book.objects.filter(title="x").delete()
    Chapter.objects.all().update(title="z")  # caught
    Note.objects.update(text="")
    Note.objects.filter(text="").delete()
    Counter.objects.update(n=0)  # caught
    Archive.objects.filter(name="a").delete()  # caught
    Archive.objects.filter(name="a").update(name="b")
    Tag.objects.filter(name="x").update(name="y")  # caught
    Tag.objects.filter(name="x").delete()
    Plain.objects.bulk_create([])  # caught
    Label.objects.update(name="")  # caught
    Label.objects.filter(name="").delete()
    books = Book.objects.filter(title="x")
    books.update(title="y")
