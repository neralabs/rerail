"""Production observations for inspection families, trigger gates and scope."""
from production_probe_runner import main

CHECK='no_source_inspection'
CASES=[dict(id=CHECK+':'+name,check=CHECK,files=[dict(path=path,source=source)]) for name,path,source in [
    ('call','tests/a.py','inspect.getsource(f)'),('lines','tests/a.py','inspect.getsourcelines(f)'),
    ('file','tests/a.py','inspect.getsourcefile(f)'),('import','tests/a.py','from inspect import getsource'),
    ('import-lines','tests/a.py','from inspect import getsourcelines'),
    ('spaced-off','tests/a.py','from   inspect import getsource'),
    ('spaced-on','tests/a.py','# from inspect import\nfrom   inspect import getsource'),
    ('other-scope','impl/a.py','inspect.getsource(f)'),('comment','tests/a.py','# inspect.getsource(f)'),
    ('cache','tests/__pycache__/a.py','inspect.getsource(f)')]]


if __name__=='__main__':main(CHECK,'rerail.inspection-production',CASES,__file__)
