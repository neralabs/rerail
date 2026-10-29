from airflow import DAG

DAG(dag_id="x")

from airflow import DAG

DAG(dag_id="x")  # noqa

from airflow import DAG

DAG(dag_id="x")  # noqa: AIR002

from airflow import DAG

DAG(dag_id="x")  # NOQA: AIR002

from airflow import DAG

DAG(dag_id="x")  # noqa: E501
