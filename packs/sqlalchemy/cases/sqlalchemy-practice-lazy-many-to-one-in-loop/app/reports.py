from sqlalchemy import select
from sqlalchemy.orm import Session, joinedload, selectinload

from app import services
from app.models import Invoice, Line, Order, User, Warehouse
from app.services import ORDER_OPTIONS


def customer_names(session: Session):
    names = []
    for order in session.scalars(select(Order).where(Order.total > 10)):
        names.append(order.customer.name)  # caught
    return names


def legacy_foreign_keys(session: Session):
    for w in session.query(Warehouse).all():
        print(w.region)  # caught
        print(w.manager)  # caught
        print(w.keeper)  # caught
        print(w.staff)


def backref_of_a_collection(session: Session):
    return [inv.region for inv in session.scalars(select(Invoice))]  # caught


def unknown_direction(session: Session):
    for line in session.scalars(select(Line).where(Line.qty > 1)):
        print(line.product)  # caught


def through_callee(session: Session):
    for order in services.get_all(db_session=session):
        print(order.customer)  # caught
    for line in services.big_lines(session):
        print(line.order)  # caught


def numbered(session: Session):
    return [f"{i}:{o.customer.name}" for i, o in enumerate(session.scalars(select(Order)))]  # caught


def pairs(session: Session):
    for line, user in session.execute(select(Line, User).join(Line.order).join(Order.customer)):
        print(user.name, line.order)  # caught


def label(order: Order) -> str:
    return order.customer.name


def labels(session: Session):
    return [label(o) for o in session.scalars(select(Order))]  # caught


# ---- left alone -------------------------------------------------------------------------------------------------


def one_parent(session: Session, order_id: int):
    out = []
    for line in session.scalars(select(Line).where(Line.order_id == order_id)):
        out.append(f"{line.order.id}:{line.id}")
    for line in session.query(Line).filter_by(order_id=order_id):
        out.append(line.order.total)
    for line in services.lines_of(session, order_id):
        out.append(line.order.total)
    return out


def one_manager(session: Session, manager_id: int):
    for w in session.scalars(select(Warehouse).where(Warehouse.manager_id == manager_id)):
        print(w.manager)


def collections(session: Session):
    for order in session.scalars(select(Order)):
        print(order.lines)
    for u in session.scalars(select(User)):
        print(u.orders)


def loaded(session: Session):
    for order in session.scalars(select(Order).options(joinedload(Order.customer))):
        print(order.customer)
    for order in session.scalars(select(Order).options(*ORDER_OPTIONS)):
        print(order.customer)
    for line in session.scalars(select(Line).options(selectinload("*"))):
        print(line.order)


def eager_by_default(session: Session):
    for order in session.scalars(select(Order)):
        print(order.coupon)


def single_row(session: Session):
    order = session.scalars(select(Order)).first()
    print(order.customer)


def not_rows(orders: list[Order]):
    for order in orders:
        print(order.customer)
