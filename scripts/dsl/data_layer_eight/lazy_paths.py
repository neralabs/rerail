# SPDX-License-Identifier: Apache-2.0
"""Path classifications shared by source and supplied-fact producers."""
import re
from data_layer_six.common import q,literals


def classifications(alias,values):
    path=alias+'.__path';qualname=alias+'.qualname'
    prefixes=q('^(?:'+'|'.join(map(re.escape,sorted(values['LAZY_SENSITIVE_PREFIXES'])))+')')
    return (f"rx_matches({path},{prefixes}) OR {path} IN "+literals(values['LAZY_SENSITIVE_EXACT_FILES'])+f" OR rx_matches({path},'^(?:l7200_versioned_transactions/_lint_|impl/common/l7300_revision_history/_lint_)') sensitive,\n"
      +f"rx_matches(lower({path}||' '||{qualname}),'bootstrap|ddl') bootstrap,\n"
      +f"rx_matches(lower({path}||' '||{qualname}),'reconcile|reconciliation|schema_ensure') OR {path} IN ('l2000_connection/schema_transaction.py','l2000_connection/table_existence_authority.py','impl/common/l2200_sql_dialect/schema_ensure.py','impl/common/l2200_sql_dialect/sql_schema_services.py','impl/common/l2200_sql_dialect/sql_core_support_free_functions.py','impl/common/l2200_sql_dialect/sql_core_support_mixin.py') reconciliation")


def function_order(alias):
    return "coalesce((SELECT group_concat(piece,'') FROM (SELECT printf('%020d/1/',d.region_start) piece FROM declarations d WHERE d.__path="+alias+".__path AND d.kind='class_definition' AND d.region_start<"+alias+".region_start AND d.__end>="+alias+".__end ORDER BY d.region_start)),'')||printf('%020d',"+alias+".region_start)"
