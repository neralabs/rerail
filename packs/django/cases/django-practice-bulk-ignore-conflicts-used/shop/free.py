"""Variants of a model no row can conflict on, reached by other names, and their reported contrasts."""

from shop.catalog import CatalogNote, Voucher
from shop.models import Grant, Label, Page, SlugPage, Ticket
from shop.models import Page as P


def alias_import(names):
    pages = P.objects.bulk_create([P(name=n) for n in names], ignore_conflicts=True)
    return [page.pk for page in pages]


def custom_manager(names):
    pages = Page.live_objects.bulk_create([Page(name=n) for n in names], ignore_conflicts=True)
    return [page.pk for page in pages]


class PageRepository:
    model = Page

    def store(self, names):
        # The receiver does not name the model; the objects do.
        pages = []
        for n in names:
            pages.append(Page(name=n))
        self.model.objects.bulk_create(pages, ignore_conflicts=True)
        return [page.pk for page in pages]


def database_key(bodies):
    tickets = Ticket.objects.bulk_create([Ticket(body=b) for b in bodies], ignore_conflicts=True)
    return [ticket.pk for ticket in tickets]


def re_exported(texts):
    notes = CatalogNote.objects.bulk_create([CatalogNote(text=t) for t in texts], ignore_conflicts=True)
    return [note.pk for note in notes]


def third_party_key(codes):
    vouchers = Voucher.objects.bulk_create([Voucher(code=c) for c in codes], ignore_conflicts=True)
    return [voucher.pk for voucher in vouchers]  # caught


def third_party_unique_field(titles):
    pages = SlugPage.objects.bulk_create([SlugPage(title=t) for t in titles], ignore_conflicts=True)
    return [page.pk for page in pages]  # caught


def constraints_by_name(page, grantees):
    grants = Grant.objects.bulk_create([Grant(page=page, grantee=g) for g in grantees], ignore_conflicts=True)
    return [grant.pk for grant in grants]  # caught


def unique_model_custom_receiver(names):
    labels = Label.objects.using("default").bulk_create([Label(name=n) for n in names], ignore_conflicts=True)
    return [label.pk for label in labels]  # caught
