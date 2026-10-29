def same_name_twice():
    try:
        work()
    except ValueError as e:
        try:
            try:
                pass
            except KeyError as e:
                raise e
        finally:
            pass


def inner_try_not_walked():
    try:
        work()
    except ValueError as e:
        try:
            pass
        except KeyError as e:
            raise e
