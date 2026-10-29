import airflow.models.dag as X
import airflow.providers.google.cloud.operators.bigquery
from airflow import DAG
from airflow.operators.bash import BashOperator
from airflow.providers.google.cloud.hooks import operators, BigQueryOperator
from airflow.providers.google import operators as ops

X(sla_miss_callback=1)
(DAG)(sla_miss_callback=1, sla_miss_callback=2)
(BashOperator)(sla=1)
airflow.providers.google.cloud.hooks.x.FooOperator(sla=1)
BigQueryOperator(sla=1)
ops.FooOperator(sla=1)
from airflow.providers.google.cloud import hooks as operators

operators.FooOperator(sla=1)
from airflow.providers.google.x import (  # operators
    BarOperator,
)

BarOperator(sla=1)
