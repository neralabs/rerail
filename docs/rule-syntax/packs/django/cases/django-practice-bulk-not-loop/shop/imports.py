from shop.models import Book, Tag


def import_titles(rows):
    created = []
    for row in rows:
        Book.objects.create(title=row["title"])  # caught
        created.append(Tag.objects.create(name=row["tag"]))  # caught
    return created


def reprice(books, factor):
    for book in books:
        book.price = book.price * factor
        book.save(update_fields=["price"])  # caught


def mark(pairs):
    for i, book in enumerate(pairs):
        book.position = i
        book.save()  # caught


def lookups(ids, slugs):
    for pk in ids:
        book = Book.objects.get(pk=pk)  # caught
        print(book)
    for slug in slugs:
        first = Book.objects.filter(slug=slug).first()  # caught
        print(first)
    return [Book.objects.get(pk=pk) for pk in ids]  # caught


def left_alone(rows, books, other):
    for row in rows:
        # the new row is used at once
        book = Book.objects.create(title=row)
        book.tags.add(*row.tags)
    for book in books:
        # not the loop's own target
        other.save()
    for pk in rows:
        # the lookup does not depend on the loop
        default = Book.objects.get(slug="default")
        print(default, pk)
    for book in books:
        # runs later, when called
        def save_later():
            book.save()
        save_later()
    for pk in rows:
        # `pk` here is the keyword's name, not the loop variable
        Book.objects.filter(pk=1).first()


def bulk(rows, books):
    Book.objects.bulk_create([Book(title=r) for r in rows])
    Book.objects.bulk_update(books, ["price"])


def per_model(models, pairs, ids, forms, state):
    for model in models:
        # each iteration queries another table
        model.objects.get(pk=1)
        model.objects.create(name="x")
    for field, model_cls in pairs:
        model_cls.objects.get(pk=field)
    for pk in ids:
        # one row lock per iteration is the point
        Book.objects.select_for_update(skip_locked=True).get(pk=pk)
        Book.objects.select_for_update().filter(pk=pk).first()
    for form in forms:
        # Model.save() takes no positional argument: this is some other save
        form.save(state)


def per_iteration_model(apps, roles):
    for role in roles:
        # the model differs per iteration
        object_cls = apps.get_model(f"main.{role.model}")
        object_cls.objects.get(pk=role.object_id)
        Book.objects.get(pk=role.object_id)  # caught


def per_database(pk, aliases, ids):
    for alias in aliases:
        # one row per database: no in_bulk or __in joins them
        Book.objects.using(alias).get(pk=pk)
    for pk in ids:
        Book.objects.using("replica").get(pk=pk)  # caught


def latest_per_author(authors):
    out = {}
    for author in authors:
        # the newest row per author: a Subquery or window query, not in_bulk
        out[author.pk] = Book.objects.filter(author=author).order_by("-pk").first()
        out[author.slug] = Book.objects.filter(author=author).first()  # caught
    return out


def per_field(value, fields):
    for field in fields:
        # another column each iteration
        Book.objects.filter(**{field: value}).first()


def counters(books, n):
    for book in books:
        # a counter moved from its old value: read-modify-write gives the F() fix
        book.hits -= 1
        book.save()
    for book in books:
        book.hits = book.hits + n
        book.save(update_fields=["hits"])
    for book in books:
        book.hits = n + 1
        book.save()  # caught
    for book in books:
        # a string built on its old value is no counter: bulk_update is right
        book.title = book.title + " (2nd ed.)"
        book.save()  # caught
    for book in books:
        # price is a DecimalField of Book: a counter
        book.price = book.price - n
        book.save()
