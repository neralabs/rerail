from pathlib import Path


async def single():
    p = Path("x")
    p.exists()


async def rebound():
    p = 1
    p = Path("x")
    p.exists()


async def read_before_rebinding():
    p = Path("x")
    p.exists()
    p = 1


async def walrus_first():
    if (p := 1):
        pass
    p = Path("x")
    p.exists()


async def except_first():
    try:
        pass
    except Exception as p:
        pass
    p = Path("x")
    p.exists()


async def match_first(v):
    match v:
        case p:
            pass
    p = Path("x")
    p.exists()


async def comprehension_walrus_first():
    [(p := 1) for _ in range(1)]
    p = Path("x")
    p.exists()


async def comprehension_target_first():
    [p for p in range(1)]
    p = Path("x")
    p.exists()


async def for_first():
    for p in []:
        pass
    p = Path("x")
    p.exists()


async def with_first():
    with open("x") as p:
        pass
    p = Path("x")
    p.exists()


async def import_first():
    import p
    p = Path("x")
    p.exists()


async def def_first():
    def p():
        pass
    p = Path("x")
    p.exists()


async def class_first():
    class p:
        pass
    p = Path("x")
    p.exists()


async def parameter_first(p):
    p = Path("x")
    p.exists()


async def annotated_parameter(p: Path):
    p.exists()


async def annotated(q=None):
    p: Path = make()
    p.exists()


async def global_first():
    global G
    G = Path("x")
    G.exists()


async def nonlocal_after():
    p = Path("x")

    def inner():
        nonlocal p
        p = 1

    p.exists()


async def augmented():
    p = Path("x")
    p += 1
    p.exists()


async def unpacked():
    a, p = 1, Path("x")
    p.exists()


M = Path("x")


async def module_single():
    M.exists()


N = Path("x")
N = Path("y")


async def module_rebound():
    N.exists()
