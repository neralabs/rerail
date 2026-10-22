from airflow.log.x import FooHook, FooBackend, FooAuthManager
from airflow.secrets.x import BarHook, BarTaskHandler, BarAuthManager
from airflow.auth.manager.x import BazHook, BazBackend, BazTaskHandler
from airflow.providers.p.auth_manager.y import QuxBackend
from airflow.providers.p.log.y import QuxAuthManager
from airflow.datasets.secrets import DsBackend

FooHook(filename_template="x")
FooBackend(filename_template="x")
FooAuthManager(filename_template="x")
BazHook(1)
BazBackend(1)
BazTaskHandler(1)
QuxBackend(1)
QuxAuthManager(filename_template="x")
b = BarHook()
b.get_conn_uri()
b.get_connections()
c = BarTaskHandler()
c.get_conn_uri()
d = BarAuthManager()
d.get_conn_uri()
e = BazBackend()
e.is_authorized_dataset()
f = DsBackend()
f.get_conn_uri()
f.iter_datasets()
