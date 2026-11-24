from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.orm import Load, Session, joinedload, selectinload

from app.models import shop
from app.models.shop import GiftOrder, Order, Tag, User
from app.services import order_service


def customer_names(session: Session):
    names = []
    for order in session.scalars(select(Order).where(Order.total > 10)):
        names.append(order.customer.name)  # caught
    return names


def legacy_query(session: Session):
    orders = session.query(Order).filter(Order.total > 10).all()
    for o in orders:
        print(o.tags)  # caught
        print(o.tags, o.customer)  # caught


async def async_rows(s: AsyncSession):
    users = (await s.scalars(select(User))).all()
    total = 0
    for u in users:
        total += len(u.orders)  # caught
    return total


def comprehension(session: Session):
    return [u.orders for u in session.scalars(select(User))]  # caught


def backref_collection(session: Session):
    for tag in session.execute(select(Tag)).scalars():
        print(tag.label, len(tag.orders))  # caught


def inherited(session: Session):
    for gift in session.scalars(select(shop.GiftOrder)).all():
        print(gift.message, gift.customer)  # caught


def through_callee(session: Session):
    for order in order_service.get_all(db_session=session):
        print(order.customer.name)  # caught
    for order in order_service.get_open(db_session=session):
        print(order.tags)  # caught
    orders = order_service.list_orders(session)
    return {o.id: o.customer for o in orders}  # caught


def statement_in_name(session: Session):
    stmt = select(Order).where(Order.total > 10)
    for order in session.execute(stmt).scalars().all():
        print(order.customer)  # caught


def through_classmethod(session: Session):
    open_orders = Order.open_orders(session)
    return [o.customer.name for o in open_orders]  # caught


# ---- left alone -------------------------------------------------------------------------------------------------


def statement_in_name_loaded(session: Session):
    stmt = select(Order).where(Order.total > 10)
    stmt = stmt.options(selectinload(Order.customer))
    for order in session.scalars(stmt):
        print(order.customer)


def statement_in_name_streamed(session: Session):
    stmt = select(Order)
    stmt = stmt.execution_options(yield_per=1000)
    for order in session.scalars(stmt):
        print(order.customer)


def loaded_by_option(session: Session):
    for order in session.scalars(select(Order).options(selectinload(Order.customer))):
        print(order.customer.name)


def loaded_by_chain(session: Session):
    stmt_rows = session.scalars(select(User).options(selectinload(User.orders).joinedload(Order.customer)))
    for u in stmt_rows:
        print(u.orders)


def loaded_by_load(session: Session):
    for order in session.query(Order).options(Load(Order).joinedload(Order.tags)):
        print(order.tags)


def loaded_by_wildcard(session: Session):
    for order in session.scalars(select(Order).options(selectinload("*"))):
        print(order.customer, order.tags)


def loaded_by_name(session: Session):
    for order in session.query(Order).options(joinedload("customer")):
        print(order.customer)


def option_added_later(session: Session):
    query = session.query(Order)
    query = query.options(selectinload(Order.tags))
    for order in query.all():
        print(order.tags)


def eager_strategies(session: Session):
    for u in session.scalars(select(User)):
        print(u.profile, u.audit)
    for order in session.scalars(select(Order)):
        print(order.lines)


def columns_and_keys(session: Session):
    for order in session.scalars(select(Order)):
        print(order.total, order.customer_id)
        order.customer = None


def single_row(session: Session):
    order = session.scalars(select(Order)).first()
    print(order.customer)
    one = session.get(Order, 1)
    print(one.tags)


def deferred_code(session: Session):
    callbacks = []
    for order in session.scalars(select(Order)):
        callbacks.append(lambda: order.customer)

        def later():
            return order.tags

        callbacks.append(later)
    return callbacks


def streamed(session: Session):
    for order in session.scalars(select(Order).execution_options(yield_per=500)):
        print(order.customer)
    for order in session.query(Order).yield_per(500):
        print(order.customer)
    for order in session.scalars(select(Order)).yield_per(500):
        print(order.customer)


def callee_left_alone(session: Session):
    for order in order_service.get_all_with_customers(db_session=session):
        print(order.customer.name)
    for order in order_service.get_built(db_session=session):
        print(order.customer.name)
    first = order_service.get_first(db_session=session)
    print(first.customer)


def not_rows(orders: list[Order], users):
    for order in orders:
        print(order.customer)
    for u in users:
        print(u.orders)


def shadowed(session: Session, others):
    for order in session.scalars(select(Order)):
        print([order.customer for order in others])


def iterable_only(session: Session):
    return [o for o in session.scalars(select(Order)) if o.total]


def lines_products(session: Session):
    for order in session.scalars(select(GiftOrder)):
        print(order.total)
