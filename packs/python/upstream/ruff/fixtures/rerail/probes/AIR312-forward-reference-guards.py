import airflow
from airflow.operators.python import PythonOperator

try:
    from airflow.providers.standard.operators.bash import BashOperator
except ImportError:
    import airflow.operators.bash.BashOperator as BashOperator


def load():
    try:
        from airflow.providers.standard.operators.python import PythonOperator
    except ImportError:
        from airflow.operators.python import PythonOperator
    hint: "PythonOperator" = PythonOperator()
    return hint


def run(op: "BashOperator", raw: "airflow.operators.bash.BashOperator") -> "list['PythonOperator']":
    return BashOperator(), op, raw
