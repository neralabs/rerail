from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.orm import Load, Session, joinedload, selectinload

from app.models import shop
from app.models.catalog import CatalogItem, ProductFamily
from app.models.shop import GiftOrder, Line, Order, Region, Tag, User, Warehouse
from app.services import order_service
from app.services.order_service import ORDER_OPTIONS


def tag_counts(session: Session):
    counts = []
    for order in session.scalars(select(Order).where(Order.total > 10)):
        counts.append(len(order.tags))  # caught
    return counts


def legacy_query(session: Session):
    orders = session.query(Order).filter(Order.total > 10).all()
    for o in orders:
        print(o.tags)  # caught
        print(o.tags, o.customer)


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
        print(gift.message, gift.tags)  # caught


def through_callee(session: Session):
    for order in order_service.get_all(db_session=session):
        print(order.tags)  # caught
    for order in order_service.get_open(db_session=session):
        print(order.tags)  # caught
    orders = order_service.list_orders(session)
    return {o.id: o.tags for o in orders}  # caught


def statement_in_name(session: Session):
    stmt = select(Order).where(Order.total > 10)
    for order in session.execute(stmt).scalars().all():
        print(order.tags)  # caught


def through_classmethod(session: Session):
    open_orders = Order.open_orders(session)
    return [o.tags for o in open_orders]  # caught


def numbered(session: Session):
    return [f"{i}:{len(u.orders)}" for i, u in enumerate(session.scalars(select(User)))]  # caught


def numbered_loop(session: Session):
    for i, u in enumerate(session.scalars(select(User)).all(), start=1):
        print(i, u.orders)  # caught


def pairs(session: Session):
    return [f"{u.id}{len(o.tags)}" for u, o in session.execute(select(User, Order).join(Order))]  # caught


def pairs_loop(session: Session):
    rows = session.execute(select(Order, User).join(User)).all()
    for o, u in rows:
        print(o.id, len(u.orders))  # caught


def describe(user: User) -> str:
    return f"{user.name}: {len(user.orders)}"


def described(session: Session):
    return [describe(u) for u in session.scalars(select(User))]  # caught


def described_by_keyword(session: Session):
    for u in session.scalars(select(User)):
        print(describe(user=u))  # caught


def first_with_tags(session: Session):
    # stops at the first match: the message says to leave such a loop
    for o in session.scalars(select(Order)):
        if o.tags:  # caught
            return o
    return None


def foreign_key_directions(session: Session):
    for w in session.scalars(select(Warehouse)):
        print(w.bins)  # caught
        print(w.staff)  # caught
        print(w.docks)  # caught
        print(w.labels)  # caught
        print(w.region, w.manager, w.keeper, w.zones)  # caught
    for r in session.scalars(select(Region)):
        print(r.warehouses)  # caught


def derived_table_names(session: Session):
    for family in session.scalars(select(ProductFamily)):
        print(family.catalog_items)  # caught
    for item in session.scalars(select(CatalogItem)):
        print(item.product_family)


# ---- left alone -------------------------------------------------------------------------------------------------

USER_OPTIONS = [selectinload(User.orders)]


def options_in_a_name(session: Session):
    for u in session.scalars(select(User).options(*USER_OPTIONS)):
        print(u.orders)
    for o in session.scalars(select(Order).options(*ORDER_OPTIONS)):
        print(o.tags)


def options_from_a_parameter(session: Session, options):
    for u in session.scalars(select(User).options(*options)):
        print(u.orders)


def many_to_one_reads(session: Session):
    for order in session.scalars(select(Order)):
        print(order.customer.name)
    for line in session.scalars(select(Line)):
        print(line.product)


def statement_in_name_loaded(session: Session):
    stmt = select(Order).where(Order.total > 10)
    stmt = stmt.options(selectinload(Order.tags))
    for order in session.scalars(stmt):
        print(order.tags)


def statement_in_name_streamed(session: Session):
    stmt = select(Order)
    stmt = stmt.execution_options(yield_per=1000)
    for order in session.scalars(stmt):
        print(order.tags)


def loaded_by_option(session: Session):
    for order in session.scalars(select(Order).options(selectinload(Order.tags))):
        print(order.tags)


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
    for order in session.query(Order).options(joinedload("tags")):
        print(order.tags)


def eager_strategies(session: Session):
    for u in session.scalars(select(User)):
        print(u.profile, u.audit)
    for order in session.scalars(select(Order)):
        print(order.lines)


def columns_and_keys(session: Session):
    for order in session.scalars(select(Order)):
        print(order.total, order.customer_id)
        order.tags = []


def single_row(session: Session):
    order = session.scalars(select(Order)).first()
    print(order.tags)
    one = session.get(Order, 1)
    print(one.tags)


def deferred_code(session: Session):
    callbacks = []
    for order in session.scalars(select(Order)):
        callbacks.append(lambda: order.tags)

        def later():
            return order.tags

        callbacks.append(later)
    return callbacks


def streamed(session: Session):
    for order in session.scalars(select(Order).execution_options(yield_per=500)):
        print(order.tags)
    for order in session.query(Order).yield_per(500):
        print(order.tags)
    for order in session.scalars(select(Order)).yield_per(500):
        print(order.tags)


def callee_left_alone(session: Session):
    for order in order_service.get_all_with_tags(db_session=session):
        print(order.tags)
    for order in order_service.get_built(db_session=session):
        print(order.tags)
    first = order_service.get_first(db_session=session)
    print(first.tags)


def not_rows(orders: list[Order], users):
    for order in orders:
        print(order.tags)
    for u in users:
        print(u.orders)
    for i, u in enumerate(users):
        print(u.orders)


def shadowed(session: Session, others):
    for order in session.scalars(select(Order)):
        print([order.tags for order in others])


def iterable_only(session: Session):
    return [o for o in session.scalars(select(Order)) if o.total]


def helper_reads_a_column(session: Session):
    return [name_of(u) for u in session.scalars(select(User))]


def name_of(user: User) -> str:
    return user.name


def latest_version(user: User) -> User:
    return user


def describe_latest(user: User) -> str:
    # the parameter is rebound before the read: the read is on another object
    user = latest_version(user)
    return str(len(user.orders))


def described_latest(session: Session):
    return [describe_latest(u) for u in session.scalars(select(User))]
