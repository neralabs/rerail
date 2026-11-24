import re
from typing import ClassVar

from sqlalchemy import text
from sqlalchemy.orm import Session

from app.models import Recipe


class SearchMixin:
    _similarity_threshold: ClassVar[float] = 0.5


class RecipeSearch(SearchMixin):
    def __init__(self, session: Session, table: str):
        self.session = session
        self.table = table

    def fuzzy(self):
        self.session.execute(text(f"SET pg_trgm.similarity_threshold = {self._similarity_threshold}"))

    def count(self):
        return self.session.execute(text(f"SELECT count(*) FROM {self.table}")).scalar()  # caught


def where_owner(owner):
    return text(f"owner = '{owner}'")  # caught


def list_for(session: Session, request_owner: str):
    return session.execute(where_owner(request_owner)).all()


def match_sql(table: str, key: str):
    return text(f"EXISTS (SELECT 1 FROM {table} WHERE body LIKE :{key})")


def search(session: Session, terms: list[str]):
    for index, term in enumerate(terms):
        key = f"term_{index}"
        session.execute(match_sql("recipe", key), {key: f"%{term}%"})


def column_def(columns, column, default):
    return default if column not in columns else column


def copy_columns(session: Session, columns):
    session.execute(text(f"INSERT INTO recipe_bak SELECT {column_def(columns, 'name', 'null')} FROM recipe"))


def clear_json(session: Session, quote):
    rows = session.execute(text("SELECT table_name, column_name FROM information_schema.columns")).all()
    for table, column in rows:
        table, column = quote(table), quote(column)
        session.execute(text(f"UPDATE {table} SET {column} = NULL"))


def tagged(dialect: str, tag_id: str):
    tags, tag = (
        ("json_each(recipe.meta, '$.tags') AS tag", "tag.value")
        if dialect == "sqlite"
        else ("json_array_elements_text(recipe.meta->'tags') AS tag", "tag")
    )
    return text(f"EXISTS (SELECT 1 FROM {tags} WHERE {tag} = :tag)").bindparams(tag=tag_id)


def authors(session: Session):
    def author(path):
        return f"message#>>'{path}'"

    return session.execute(text(f"SELECT {author('{user_id}')} FROM messages")).all()


def participants(session: Session, dialect: str):
    if dialect == "sqlite":

        def author(path):
            return f"json_extract(message, '{path}')"

        author_id = f"COALESCE({author('$.user_id')}, owner_id)"
    else:

        def author(path):
            return f"message#>>'{path}'"

        author_id = f"COALESCE({author('{user_id}')}, owner_id)"
    return session.execute(text(f"SELECT DISTINCT {author_id} FROM messages")).all()


SORTS = {"new": "created_at DESC", "name": "name ASC"}


def sorted_by(session: Session, sort: str):
    return session.execute(text(f"SELECT * FROM recipe ORDER BY {SORTS[sort]}")).all()


def ordered(session: Session, column: str):
    if column not in ("name", "created_at"):
        raise ValueError(column)
    return session.execute(text(f"SELECT * FROM recipe ORDER BY {column}")).all()


def ensure_index(session: Session):
    from app.config import INDEX_METHOD

    if INDEX_METHOD:
        session.execute(text(f"CREATE INDEX IF NOT EXISTS recipe_vec ON recipe USING {INDEX_METHOD} (vec)"))


def owners(session: Session, ids: list[int]):
    owner_id = "COALESCE(owner, 'system')"
    query = text(f"SELECT id, {owner_id} FROM recipe WHERE id IN :ids")
    result = {}
    for item_id, owner_id in session.execute(query, {"ids": ids}).all():
        result[item_id] = owner_id
    return result


def owners_loop(session: Session, rows):
    clause = "1 = 1"
    for row in rows:
        session.execute(text(f"DELETE FROM recipe WHERE {clause}"))  # caught
        clause = row.clause


def _index_config():
    from app.config import INDEX_METHOD, IVFFLAT_LISTS

    method = INDEX_METHOD or "ivfflat"
    return method, f"WITH (lists = {IVFFLAT_LISTS})"


def build_index(session: Session):
    method, options = _index_config()
    session.execute(text(f"CREATE INDEX IF NOT EXISTS recipe_vec ON recipe USING {method} (vec) {options}"))


def by_metadata(session: Session, key: str, value: str):
    if not re.fullmatch(r"[a-zA-Z0-9_.]+", key):
        raise ValueError(key)
    return session.execute(text(f"SELECT id FROM docs WHERE metadata->>'$.{key}' = :value"), {"value": value}).all()


def by_metadata_unchecked(session: Session, key: str, value: str):
    return session.execute(text(f"SELECT id FROM docs WHERE metadata->>'$.{key}' = :value"), {"value": value}).all()  # caught


def day_of(field):
    return f"DATE({field})"


def per_day(session: Session):
    return session.execute(text(f"SELECT {day_of('created_at')}, count(*) FROM recipe GROUP BY 1")).all()


def per_day_of(session: Session, column: str):
    return session.execute(text(f"SELECT {day_of(column)}, count(*) FROM recipe GROUP BY 1")).all()  # caught


def owner_join(session: Session, owner: str):
    condition = str((Recipe.owner == owner).compile(dialect=session.get_bind().dialect, compile_kwargs={"literal_binds": True}))
    return session.execute(text(f"SELECT * FROM recipe JOIN owner ON {condition}")).all()
