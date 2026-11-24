from sqlalchemy import text


def rename_backup(engine, table_back_name: str):
    with engine.begin() as connection:
        connection.execute(text(f"ALTER TABLE recipe RENAME TO {table_back_name}"))
