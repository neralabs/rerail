import airflow.providers

a = (airflow.providers.google.operators  # comment
     .bq.Op)(task_id="b")
c = (airflow.providers.google
     .operators.bq.Op)(task_id="d")
e = airflow.providers.google. operators .bq.Op(task_id="f")
import airflow
g = (airflow.providers.google).operators.bq.Op(task_id="h")
i = (airflow.providers.google.operators).bq.Op(task_id="j")
k = airflow.providers.google.operators.Op.method(task_id="l")
m = airflow.providers.operators.hooks.x.y(task_id="n")
