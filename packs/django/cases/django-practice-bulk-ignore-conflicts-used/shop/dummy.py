from shop.models import Member, Membership, Note, Page, ProjectPage, Seat, Tenant
from shop.tasks import send_invite


def create_pages(names):
    # Page's only unique constraint is its generated UUID key: no object can conflict, they are the stored rows.
    pages = [Page(name=n) for n in names]
    pages = Page.objects.bulk_create(pages, ignore_conflicts=True)
    ProjectPage.objects.bulk_create([ProjectPage(page=page) for page in pages], batch_size=1000)
    return pages


def page_ids(names):
    pages = create_pages(names)
    return [page.id for page in pages]


def count_notes(texts):
    created = Note.objects.bulk_create([Note(text=t) for t in texts], ignore_conflicts=True)
    return len(created)


def invite_members(emails):
    # Member inherits unique_together from MemberBase (shop/base.py).
    members = [Member(email=e) for e in emails]
    Member.objects.bulk_create(members, ignore_conflicts=True)
    for member in members:  # caught
        send_invite.delay(member.email, member.id)


def seat_ids(labels):
    seats = Seat.objects.bulk_create([Seat(owner=label) for label in labels], ignore_conflicts=True)
    return [seat.pk for seat in seats]  # caught


def membership_ids(labels):
    rows = Membership.objects.bulk_create(
        [Membership(label=label, project="p") for label in labels], ignore_conflicts=True
    )
    return [row.pk for row in rows]  # caught


def tenant_ids(titles):
    tenants = Tenant.objects.bulk_create([Tenant(title=t) for t in titles], ignore_conflicts=True)
    return [tenant.pk for tenant in tenants]  # caught
