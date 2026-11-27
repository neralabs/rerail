from pickled.celery import app
from pickled.models import Invoice


@app.task
def bill(payload):
    return payload


def helper(x):
    return x


def send(session, pk, path):
    invoice = session.get(Invoice, pk)
    bill.delay(invoice)  # caught
    bill.delay(session)  # caught
    bill.delay({1, 2})
    bill.delay(frozenset([pk]))
    bill.delay(helper)
    bill.delay(lambda x: x)  # caught
    bill.delay(open(path))  # caught
    bill.delay(x for x in [pk])  # caught
