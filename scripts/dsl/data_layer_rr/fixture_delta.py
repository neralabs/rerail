"""Apply an exact physical fixture delta while preserving untouched entries."""
from pathlib import Path
import shutil
from replay import write_fixture


def apply(root,previous,current):
    root=Path(root)
    changed={p for p in previous.keys()|current.keys() if previous.get(p)!=current.get(p)}
    for name in sorted(previous.keys()&changed,key=lambda p:p.count('/'),reverse=True):
        path=root/name
        if path.is_dir() and not path.is_symlink():shutil.rmtree(path)
        else:path.unlink()
        parent=path.parent
        while parent!=root:
            if current.get(parent.relative_to(root).as_posix(),{}).get('directory'):break
            try:parent.rmdir()
            except OSError:break
            parent=parent.parent
    write_fixture(root,[f for p,f in current.items() if p in changed])
    return sorted(changed)
