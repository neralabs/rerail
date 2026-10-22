from airflow.operators.python import PythonOperator

PythonOperator(task_id="t", python_callable=print)

from airflow.operators.python import PythonOperator

PythonOperator(task_id="t", python_callable=print)  # noqa

from airflow.operators.python import PythonOperator

PythonOperator(task_id="t", python_callable=print)  # noqa: AIR312

from airflow.operators.python import PythonOperator

PythonOperator(task_id="t", python_callable=print)  # NOQA: AIR312

from airflow.operators.python import PythonOperator

PythonOperator(task_id="t", python_callable=print)  # noqa: E501
