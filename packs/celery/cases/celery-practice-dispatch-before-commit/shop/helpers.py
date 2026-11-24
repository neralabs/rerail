from shop.tasks import ship


def queue_shipping(order_id):
    ship.delay(order_id)


def queue_deep(order_id):
    queue_shipping(order_id)


def queue_after_own_commit(session, order_id):
    session.commit()
    ship.delay(order_id)
