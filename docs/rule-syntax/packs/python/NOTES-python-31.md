# Python translation batch 31

## `ruff-fbt001-boolean-type-hint-positional-argument`

Follows Ruff and the JSON. Parentheses do not stop a property-setter exemption (`@(flag).setter`), and a class binding named `bool` stops later method annotations from resolving to the builtin (`class C: def bool(self): ...; def f(self, flag: bool): ...`). Ruff reports the otherwise unusual sole tuple argument `Union[((bool, int))]`, so the translation follows it without treating a tuple that is one of several union arguments as a boolean annotation.

## `ruff-fbt002-boolean-default-value-positional-argument`

Follows Ruff and the JSON. `@(flag).setter\ndef flag(self, value=True): ...` is exempt just like `@flag.setter`; the existing boolean-trap library did not remove parentheses around the decorator and property expression.

## `ruff-fbt003-boolean-positional-value-in-call`

Follows Ruff and the JSON into recursively parsed string annotations. For example, `x: "Annotated[int, use(True)]"` reports FBT003, while the same text in an ordinary string does not.

## `ruff-log004-log-exception-outside-except-handler`

Follows Ruff and the JSON into recursively parsed string annotations. For example, `x: "logger.exception('missing')"` reports LOG004; the host annotation's exception-handler context remains authoritative.

## `ruff-log007-exception-without-exc-info`

Follows Ruff and the JSON into recursively parsed string annotations. For example, `x: "logger.exception('missing', exc_info=False)"` reports LOG007. Ruff also treats `f"{'a' f''}"` as falsy for this rule; the translation preserves that observed behavior rather than Python runtime truthiness.

## `ruff-log014-exc-info-outside-except-handler`

Follows Ruff and the JSON into recursively parsed string annotations. For example, `x: "logger.info('missing', exc_info=True)"` reports LOG014; a matching annotation hosted inside an `except` clause remains exempt.

## Library and engine follow-ups

`python/python-31.rerail` adds `prop_setter` for FBT001 and FBT002, plus the string-type-expression and guest-call vocabulary shared by FBT003, LOG004, LOG007 and LOG014. After integration, move setter parenthesis handling into `python/misc-boolean-trap.rerail`; extend `python/typing.rerail`'s cast type root to accept `cast(typ="T", val=x)`; and fold the demand-driven guest call relations into `python/ruff.rerail` and `python/logging.rerail`. Existing libraries were intentionally not edited. No engine primitive is needed.
