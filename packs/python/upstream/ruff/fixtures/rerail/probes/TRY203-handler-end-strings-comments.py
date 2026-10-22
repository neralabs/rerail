def quote_in_trailing_comment():
    try:
        work()
    except ValueError:
        raise
        log('it\'s # here')  # say "hi"


def hash_in_triple_quoted_string():
    try:
        work()
    except ValueError as e:
        raise e
        x = """
    # not a comment"""


def trailing_semicolon():
    try:
        work()
    except ValueError as e:
        raise e ;  # don't
