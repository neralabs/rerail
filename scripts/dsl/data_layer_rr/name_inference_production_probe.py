"""Production positives for each name family, trigger gating and filename aliases."""
from production_probe_runner import main

CHECK='serverplan_name_inference'
CASES=[dict(id=CHECK+':'+label,check=CHECK,files=[dict(path='l2070_batching/'+filename,source=text)])
    for label,filename,text in [
        ('column','a.py','column == "count"'),('parse','a.py','ref.endswith(".raw")'),
        ('synthesis','a.py','refs.get(f"{ref}.raw")'),('type','sql_column_type_hints.py','column.split("_")'),
        ('upper-off','a.py','SOURCE_REF.ENDSWITH(".RAW")'),
        ('upper-on','a.py','# ref\nSOURCE_REF.ENDSWITH(".RAW")')]]
for name,target,alias in [('type-alias','runtime.py','sql_column_type_hints.py'),
                          ('type-alias-reverse','sql_column_type_hints.py','runtime.py')]:
    CASES.append(dict(id=CHECK+':'+name,check=CHECK,files=[
        dict(path='l2070_batching/'+target,source='column.split("_")'),
        dict(path='l2070_batching/'+alias,symlink=target)]))


if __name__=='__main__':main(CHECK,'rerail.name-inference-production',CASES,__file__)
