from shop.models import IssueSequence
from shop.seed import create_issues, create_labels, create_labels_unread


def seed(titles, names):
    issues = create_issues(titles)
    for n, issue in enumerate(issues):  # caught
        IssueSequence.objects.create(issue=issue, sequence=n)
    labels = create_labels(names)
    print("label ids", [label.id for label in labels])  # caught


def seed_quietly(names):
    # The caller does not read the objects: nothing to report.
    create_labels_unread(names)
    labels = create_labels(names)
    if labels:
        print("seeded")
