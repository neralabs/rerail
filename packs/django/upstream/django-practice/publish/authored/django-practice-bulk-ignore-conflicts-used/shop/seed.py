from shop.models import Issue, Label


def create_issues(titles):
    # Hands the objects to the caller: reported where a caller uses them (shop/commands.py).
    return Issue.objects.bulk_create([Issue(title=t) for t in titles], ignore_conflicts=True)


def create_labels(names):
    labels = [Label(name=n) for n in names]
    Label.objects.bulk_create(labels, ignore_conflicts=True)
    return labels


def create_labels_unread(names):
    return Label.objects.bulk_create([Label(name=n) for n in names], ignore_conflicts=True)
