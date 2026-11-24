import sqlalchemy as sa
from sqlalchemy import select, text
from sqlalchemy import text as sql_text
from sqlalchemy.orm import Session

from app.config import PAGE_SIZE, RECIPE_TABLE, SCHEMA
from app.models import Recipe

LOCAL_TABLE = "recipe_archive"


def by_name(session: Session, name: str):
    return session.execute(text(f"SELECT * FROM recipe WHERE name = '{name}'")).all()  # caught


def sorted_recipes(session: Session, col: str, direction: str):
    stmt = select(Recipe).order_by(text(f"{col} {direction}"))  # caught
    return session.scalars(stmt).all()


def filtered(session: Session, where: str):
    return session.scalars(select(Recipe).where(sa.text(f"name = '{where}'"))).all()  # caught


def by_id_percent(session: Session, item_id):
    return session.execute(text("SELECT * FROM recipe WHERE id = %s" % item_id)).first()  # caught


def by_slug_format(session: Session, slug):
    return session.execute(text("SELECT * FROM recipe WHERE slug = '{}'".format(slug))).first()  # caught


def by_clause(session: Session, clause: str):
    return session.execute(sql_text("SELECT * FROM recipe WHERE " + clause)).all()  # caught


def delete_one(session: Session, item_id):
    sql = f"DELETE FROM recipe WHERE id = {item_id}"
    session.execute(text(sql))  # caught


def mine(session: Session, owner: str, active: bool):
    sql = "SELECT * FROM recipe WHERE 1 = 1"
    if active:
        sql += " AND active"
    sql += f" AND owner = '{owner}'"
    return session.execute(
        text(sql)  # caught
    ).all()


def pick_columns(session: Session, columns: list[str]):
    return session.execute(text("SELECT " + ", ".join(columns) + " FROM recipe")).all()  # caught


def in_schema(session: Session):
    session.execute(text(f"SET search_path TO {SCHEMA}"))  # caught


def vacuum(engine: sa.Engine, table: str):
    with engine.connect() as conn:
        conn.exec_driver_sql(f"VACUUM ANALYZE {table}")  # caught


class VectorStore:
    def __init__(self, session: Session, collection_name: str):
        self._session = session
        self._collection_name = collection_name

    def count(self):
        return self._session.execute(text(f'SELECT count(*) FROM "{self._collection_name}"')).scalar()  # caught


def with_default(session: Session, owner: str | None = None):
    if not owner:
        owner = "system"
    return session.execute(text(f"SELECT * FROM recipe WHERE owner = '{owner}'")).all()  # caught


# ---- left alone ------------------------------------------------------------------------------------------------


def constant(session: Session):
    return session.execute(text("SELECT 1")).scalar()


def bound(session: Session, name: str):
    return session.execute(text("SELECT * FROM recipe WHERE name = :name").bindparams(name=name)).all()


def bound_dict(session: Session, name: str):
    return session.execute(text("SELECT * FROM recipe WHERE name = :name"), {"name": name}).all()


def module_constants(session: Session):
    session.execute(text(f"SELECT * FROM {RECIPE_TABLE} LIMIT {PAGE_SIZE}"))
    session.execute(text(f"SELECT * FROM {LOCAL_TABLE}"))
    session.execute(text("SELECT * FROM " + LOCAL_TABLE + " LIMIT %d" % PAGE_SIZE))
    return session.execute(text(f"SELECT count(*) FROM {Recipe.__tablename__}")).scalar()


def casts(session: Session, limit, ids: list[int]):
    return session.execute(text(f"SELECT * FROM recipe LIMIT {int(limit)} OFFSET {len(ids)}")).all()


def bind_names(session: Session, values: dict[str, str]):
    for i, (key, value) in enumerate(values.items()):
        param = f"p_{i}"
        session.execute(text(f"SELECT * FROM recipe WHERE name = :{param}"), {param: value})


def in_list(session: Session, ids: list[int]):
    placeholders = ", ".join(f":id_{i}" for i in range(len(ids)))
    params = {f"id_{i}": v for i, v in enumerate(ids)}
    return session.execute(text(f"SELECT * FROM recipe WHERE id IN ({placeholders})"), params).all()


def chosen_direction(session: Session, descending: bool):
    direction = "DESC" if descending else "ASC"
    return session.scalars(select(Recipe).order_by(text(f"name {direction}"))).all()


def passed_through(session: Session, query: str):
    return session.execute(text(query)).all()


def restart_sequence(session: Session, next_id: int | None = None, step: int = 1):
    session.execute(text(f"ALTER SEQUENCE recipe_id_seq RESTART WITH {next_id} INCREMENT BY {step}"))
