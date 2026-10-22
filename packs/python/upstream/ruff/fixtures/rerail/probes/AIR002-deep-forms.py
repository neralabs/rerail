from airflow.models.dag import DAG
from airflow.sdk.definitions.dag import dag
import airflow.sdk.definitions.dag as defs

DAG(
    dag_id="x",
)
dag()(lambda: None)
defs.DAG("a")
defs.dag("b")
defs.other("c")
