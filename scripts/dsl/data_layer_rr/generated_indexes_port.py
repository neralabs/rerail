"""Transport generated-index API arguments and complete rendered findings."""
from pathlib import Path


def normalize(values, root):
    rows=[]
    for value in values:
        head,separator,message=value.partition(': ')
        path,colon,line=head.rpartition(':')
        if not separator or not colon:raise ValueError('Malformed generated-index finding')
        path=Path(path)
        if path.is_absolute():path=path.relative_to(root)
        rows.append(dict(path=path.as_posix(),line=int(line),column=None,code='missing_source',message=message,severity=None))
    return rows
