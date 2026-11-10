# Primitive values in supplied facts

Supplied facts can contain Python values that ordinary JSON cannot preserve.
For example, facts can contain non-finite floats and negative zero.
The native input reader accepts a typed envelope for these values.
Rules do not call Python to decode, format, or convert them.

Each envelope is a two-element JSON array: `[kind, payload]`.

| Kind | Payload |
| --- | --- |
| `null` | JSON `null` |
| `bool` | JSON boolean |
| `int` | Decimal integer text |
| `float` | Sixteen hexadecimal digits for the IEEE-754 binary64 bits |
| `str` | JSON string |
| `list` | Array of envelopes |
| `dict` | Ordered array of `[string_key, envelope]` pairs |

Dictionary payloads retain member order. An envelope cannot collide with a user list or dictionary.
For example, a user list containing `"float"` and a bit string still has an outer `list` envelope.

The reader provides two projections:

```rr
.decl text_value(i: number, value: symbol)
.input text_value(IO=json,filename="values.json",path="/*",projection="python-value-str")

.decl integer_value(i: number, value: symbol, error_type: symbol, detail: symbol)
.input integer_value(IO=json,filename="values.json",path="/*",projection="python-value-int")
```

The string projection implements Python `str()` for the transported primitives.
It uses Python representations for nested values, including dictionary members.
The integer projection returns canonical decimal text or a conversion error.
A successful conversion has empty error fields. A failed conversion has an empty value field.
Malformed envelopes fail the selector. The reader publishes no partial prefix.

Selectors share the reader's decoded JSON document and physical read.
The dispatch generator uses literal extractor and field names in its selectors.
It converts only `line` and `occurrence` to integers.
It formats only the four string fields consumed by the reporter.
Unrelated fact families remain in the transported document and do not enter these selectors.

The encoder in `scripts/dsl/data_layer_rr/python_value_transport.py` preserves types and float bits.
The native implementation is in `native/src/code_rule_dsl/logic/plan/corpus/inputs.rs`.
The supplied-fact acceptance corpus compares complete Python and native observations through both APIs.
