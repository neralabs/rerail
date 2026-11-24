from enum import StrEnum

from sqlalchemy import TextClause, text


class Interval(StrEnum):
    day = "day"
    week = "week"

    def sql_interval(self) -> TextClause:
        return text(f"'1 {self.value}'::interval")


class EventType(StrEnum):
    created = "recipe.created"
    deleted = "recipe.deleted"


def subscribed(event: EventType):
    return text(f"events @> '[\"{event}\"]'")


def subscribed_raw(event: str):
    return text(f"events @> '[\"{event}\"]'")  # caught


class Status(StrEnum):
    active = "active"
    trialing = "trialing"
    canceled = "canceled"

    @classmethod
    def billable(cls) -> set["Status"]:
        return {cls.active, cls.trialing}


BILLABLE_WHERE = text(
    "status IN (" + ", ".join(f"'{status}'" for status in sorted(Status.billable())) + ")"
)


def notify_all(kinds: list[EventType]):
    for kind in kinds:
        subscribed(kind)


class EngineSpec:
    _UTCNOW_SQL = {"postgresql": "now() AT TIME ZONE 'utc'", "sqlite": "datetime('now')"}

    @classmethod
    def quote_table(cls, table: str, dialect) -> str:
        return dialect.identifier_preparer.quote(table)

    @classmethod
    def show_columns(cls, conn, table: str, dialect):
        full_table_name = cls.quote_table(table, dialect)
        return conn.execute(text(f"SHOW COLUMNS FROM {full_table_name}")).fetchall()

    @classmethod
    def probe(cls, conn, url: str):
        url = url.replace('"', '""')
        return conn.execute(text(f'SELECT * FROM "{url}" LIMIT 1'))

    @classmethod
    def touch(cls, conn, dialect_name: str, task_id: int):
        expr = cls._UTCNOW_SQL.get(dialect_name)
        conn.execute(text(f"UPDATE tasks SET heartbeat = {expr} WHERE id = :id"), {"id": task_id})

    @classmethod
    def drop(cls, conn, table: str):
        conn.execute(text(f"DROP TABLE IF EXISTS {table}"))  # caught
