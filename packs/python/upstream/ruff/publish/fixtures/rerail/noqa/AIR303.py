from airflow.lineage.hook import HookLineageCollector

HookLineageCollector().create_asset("s3://bucket/key")

from airflow.lineage.hook import HookLineageCollector

HookLineageCollector().create_asset("s3://bucket/key")  # noqa

from airflow.lineage.hook import HookLineageCollector

HookLineageCollector().create_asset("s3://bucket/key")  # noqa: AIR303

from airflow.lineage.hook import HookLineageCollector

HookLineageCollector().create_asset("s3://bucket/key")  # NOQA: AIR303

from airflow.lineage.hook import HookLineageCollector

HookLineageCollector().create_asset("s3://bucket/key")  # noqa: E501
