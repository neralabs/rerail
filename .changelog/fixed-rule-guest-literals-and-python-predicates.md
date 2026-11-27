Rules no longer miss findings in parsed string annotations or compiled templates. Literal values, trailing comments and repeated annotations are handled correctly; adding unrelated files no longer drops annotation findings. Python rules gain Ruff's parser and tokenizer, plus Unicode case predicates that agree with Ruff.

Collection-call rules normalize parenthesized callees before name and binding lookup, including calls inside cast type strings.
