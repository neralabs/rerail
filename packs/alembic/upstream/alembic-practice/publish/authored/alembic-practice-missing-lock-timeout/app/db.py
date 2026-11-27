from sqlalchemy import create_engine

engine = create_engine("postgresql://app@db/app", connect_args={"options": "-c lock_timeout=5000"})
