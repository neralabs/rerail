from airflow.operators.python import PythonOperator

my_task = PythonOperator(task_id="my_other_task", python_callable=print)

from airflow.operators.python import PythonOperator

my_task = PythonOperator(task_id="my_other_task", python_callable=print)  # noqa

from airflow.operators.python import PythonOperator

my_task = PythonOperator(task_id="my_other_task", python_callable=print)  # noqa: AIR001

from airflow.operators.python import PythonOperator

my_task = PythonOperator(task_id="my_other_task", python_callable=print)  # NOQA: AIR001

from airflow.operators.python import PythonOperator

my_task = PythonOperator(task_id="my_other_task", python_callable=print)  # noqa: E501
