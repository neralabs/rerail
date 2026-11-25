from shop.tasks import notify, ship


# ---- reported: a path runs the write, then the send, before the commit --------------------------------------------


def write_in_branch_then_send(session, order, is_new):
    if is_new:
        session.add(order)
    ship.delay(order.id)  # caught
    session.commit()


def send_in_finally(session, order):
    try:
        session.add(order)
        session.flush()
        ship.delay(order.id)  # caught
    finally:
        session.commit()


def send_in_handler(session, order):
    try:
        session.add(order)
        session.flush()
    except ValueError:
        notify.delay(order.id)  # caught
    session.commit()


def commit_in_other_branch(session, order, fast):
    session.add(order)
    if fast:
        session.commit()
    else:
        ship.delay(order.id)  # caught
        session.commit()


def return_in_nested_branch(session, order, dry_run):
    if order.id:
        session.add(order)
        if dry_run:
            return None
    ship.delay(order.id)  # caught
    session.commit()


def commit_then_return_elsewhere(session, order, fast):
    session.add(order)
    if fast:
        session.commit()
        return order
    ship.delay(order.id)  # caught
    session.commit()


class Service:
    def __init__(self, session):
        self.session = session

    def notify(self, order):
        ship.delay(order.id)

    def place(self, order):
        self.session.add(order)
        self.notify(order)  # caught
        self.session.commit()


# ---- silent: no path runs the write and then the send -------------------------------------------------------------


def upsert(session, order, is_new):
    if is_new:
        session.add(order)
    else:
        ship.delay(order.id)
    session.commit()


def by_state(session, order, state):
    if state == "new":
        session.add(order)
    elif state == "paid":
        ship.delay(order.id)
    else:
        notify.delay(order.id)
    session.commit()


def by_match(session, order, command):
    match command:
        case "create":
            session.add(order)
        case "ship":
            ship.delay(order.id)
    session.commit()


def handler_or_else(session, order):
    try:
        order.validate()
    except ValueError:
        session.delete(order)
    else:
        ship.delay(order.id)
    session.commit()


def write_then_return(session, order, is_new):
    if is_new:
        session.add(order)
        return order
    ship.delay(order.id)
    session.commit()


def write_then_raise(session, order):
    if not order.id:
        session.delete(order)
        raise ValueError("no id")
    ship.delay(order.id)
    session.commit()


# ---- gap: not reported, though the task can run before the commit -------------------------------------------------


def ship_all(session, orders):
    for o in orders:
        ship.delay(o.id)  # gap: the previous iteration's add comes first, but later in the loop body
        session.add(o)
    session.commit()
