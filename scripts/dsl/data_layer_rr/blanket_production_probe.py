"""Verify normalized method names and logical-owner scope across shared physical files."""
from production_probe_runner import main
CHECK='no_blanket_grammar_bypass'
SQL='impl/common/l2200_sql_dialect/sql_connection.py'
CASES=[dict(id=CHECK+':'+label,check=CHECK,files=[dict(path=SQL,source=text)]) for label,text in [
    ('unicode-on','# execute_grammar\ndef ｅxecute_grammar(): pass\n'),
    ('unicode-off','def ｅxecute_grammar(): pass\n')]]
CASES += [
    dict(id=CHECK+':alias-from-sql',check=CHECK,files=[dict(path=SQL,source='def execute_grammar(): pass\n'),
        dict(path='impl/common/l2200_sql_dialect/alias.py',symlink='sql_connection.py')]),
    dict(id=CHECK+':alias-to-sql',check=CHECK,files=[dict(path='impl/shared.py',source='def execute_grammar(): pass\n'),
        dict(path=SQL,symlink='../../shared.py')]),
    dict(id=CHECK+':allowed-alias',check=CHECK,files=[dict(path='scripts/lint_impl_direct_execution.py',source='_ALLOW_DIRECT_EXECUTION=1\n'),
        dict(path='impl/alias.py',symlink='../scripts/lint_impl_direct_execution.py')]),
]
if __name__=='__main__':main(CHECK,'rerail.blanket-production',CASES,__file__)
