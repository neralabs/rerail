select
  unquoted_identifier,
  another as unquoted_alias,
  1 + 2 as expression_alias,
  "case_specific_reference",
  "REFERENCE WITH SPACE",
  "I-c@nn0t-be~un-quoted",
  "1_starts_with_digit",
  "SELECT",  -- Keyword as identifier
  another as "Case_Specific_Alias"
from my_schema.my_table
