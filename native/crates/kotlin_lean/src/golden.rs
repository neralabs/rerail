//! Probe texts whose tapes are tree-sitter-kotlin's (`python_tape_expect`): the lean parser's must be
//! the same.

fn same(src: &str, want: &str) {
    let got = crate::tape_text(&crate::tape(src.as_bytes()).expect("a tape"));
    assert_eq!(got, want, "{src}");
}

/// `x as a.b.C<T>` at a line's end: comparison over generic call.
#[test]
fn split_comparison_over_generic_call() {
    same(
        r###"fun f() {
  val a = x as java.util.List<Int>
  assertFailsWith<E> { y }
}
"###,
        r###"source_file [0..74]
  function_declaration [0..73]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..73]
      statements [12..71]
        property_declaration [12..71]
          binding_pattern_kind [12..15]
            val [12..15]
          variable_declaration [16..17]
            simple_identifier [16..17]
          comparison_expression [20..71]
            comparison_expression [20..43]
              navigation_expression [20..39]
                navigation_expression [20..34]
                  as_expression [20..29]
                    simple_identifier [20..21]
                    as [22..24]
                    user_type [25..29]
                      type_identifier [25..29]
                  navigation_suffix [29..34]
                    . [29..30]
                    simple_identifier [30..34]
                navigation_suffix [34..39]
                  . [34..35]
                  simple_identifier [35..39]
              < [39..40]
              simple_identifier [40..43]
            > [43..44]
            call_expression [47..71]
              simple_identifier [47..62]
              call_suffix [62..71]
                type_arguments [62..65]
                  type_projection [63..64]
                    user_type [63..64]
                      type_identifier [63..64]
                annotated_lambda [66..71]
                  lambda_literal [66..71]
                    statements [68..69]
                      simple_identifier [68..69]
"###,
    );
}

/// `x as a.b.C<T>` at a line's end: comparison over call.
#[test]
fn split_comparison_over_call() {
    same(
        r###"fun f() {
  val a = x as java.util.List<Int>
  foo()
}
"###,
        r###"source_file [0..55]
  function_declaration [0..54]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..54]
      statements [12..52]
        property_declaration [12..52]
          binding_pattern_kind [12..15]
            val [12..15]
          variable_declaration [16..17]
            simple_identifier [16..17]
          comparison_expression [20..52]
            comparison_expression [20..43]
              navigation_expression [20..39]
                navigation_expression [20..34]
                  as_expression [20..29]
                    simple_identifier [20..21]
                    as [22..24]
                    user_type [25..29]
                      type_identifier [25..29]
                  navigation_suffix [29..34]
                    . [29..30]
                    simple_identifier [30..34]
                navigation_suffix [34..39]
                  . [34..35]
                  simple_identifier [35..39]
              < [39..40]
              simple_identifier [40..43]
            > [43..44]
            call_expression [47..52]
              simple_identifier [47..50]
              call_suffix [50..52]
                value_arguments [50..52]
"###,
    );
}

/// `x as a.b.C<T>` at a line's end: unqualified type kept.
#[test]
fn split_unqualified_type_kept() {
    same(
        r###"fun f() {
  val a = x as List<Int>
  assertFailsWith<E> { y }
}
"###,
        r###"source_file [0..64]
  function_declaration [0..63]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..63]
      statements [12..61]
        property_declaration [12..34]
          binding_pattern_kind [12..15]
            val [12..15]
          variable_declaration [16..17]
            simple_identifier [16..17]
          as_expression [20..34]
            simple_identifier [20..21]
            as [22..24]
            user_type [25..34]
              type_identifier [25..29]
              type_arguments [29..34]
                type_projection [30..33]
                  user_type [30..33]
                    type_identifier [30..33]
        call_expression [37..61]
          simple_identifier [37..52]
          call_suffix [52..61]
            type_arguments [52..55]
              type_projection [53..54]
                user_type [53..54]
                  type_identifier [53..54]
            annotated_lambda [56..61]
              lambda_literal [56..61]
                statements [58..59]
                  simple_identifier [58..59]
"###,
    );
}

/// `x as a.b.C<T>` at a line's end: declaration keeps type.
#[test]
fn split_declaration_keeps_type() {
    same(
        r###"fun f() {
  val a = x as java.util.List<Int>
  val b = 1
}
"###,
        r###"source_file [0..59]
  function_declaration [0..58]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..58]
      statements [12..56]
        property_declaration [12..44]
          binding_pattern_kind [12..15]
            val [12..15]
          variable_declaration [16..17]
            simple_identifier [16..17]
          as_expression [20..44]
            simple_identifier [20..21]
            as [22..24]
            user_type [25..44]
              type_identifier [25..29]
              type_identifier [30..34]
              type_identifier [35..39]
              type_arguments [39..44]
                type_projection [40..43]
                  user_type [40..43]
                    type_identifier [40..43]
        property_declaration [47..56]
          binding_pattern_kind [47..50]
            val [47..50]
          variable_declaration [51..52]
            simple_identifier [51..52]
          integer_literal [55..56]
"###,
    );
}

/// `x as a.b.C<T>` at a line's end: two arguments keep type.
#[test]
fn split_two_arguments_keep_type() {
    same(
        r###"fun f() {
  val a = x as java.util.Map<Int, Int>
  assertFailsWith<E> { y }
}
"###,
        r###"source_file [0..78]
  function_declaration [0..77]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..77]
      statements [12..75]
        property_declaration [12..48]
          binding_pattern_kind [12..15]
            val [12..15]
          variable_declaration [16..17]
            simple_identifier [16..17]
          as_expression [20..48]
            simple_identifier [20..21]
            as [22..24]
            user_type [25..48]
              type_identifier [25..29]
              type_identifier [30..34]
              type_identifier [35..38]
              type_arguments [38..48]
                type_projection [39..42]
                  user_type [39..42]
                    type_identifier [39..42]
                type_projection [44..47]
                  user_type [44..47]
                    type_identifier [44..47]
        call_expression [51..75]
          simple_identifier [51..66]
          call_suffix [66..75]
            type_arguments [66..69]
              type_projection [67..68]
                user_type [67..68]
                  type_identifier [67..68]
            annotated_lambda [70..75]
              lambda_literal [70..75]
                statements [72..73]
                  simple_identifier [72..73]
"###,
    );
}

/// `x as a.b.C<T>` at a line's end: call across line.
#[test]
fn split_call_across_line() {
    same(
        r###"fun f() {
  val a = x as java.util.List<Int>
  (1)
}
"###,
        r###"source_file [0..53]
  function_declaration [0..52]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..52]
      statements [12..50]
        property_declaration [12..50]
          binding_pattern_kind [12..15]
            val [12..15]
          variable_declaration [16..17]
            simple_identifier [16..17]
          call_expression [20..50]
            navigation_expression [20..39]
              navigation_expression [20..34]
                as_expression [20..29]
                  simple_identifier [20..21]
                  as [22..24]
                  user_type [25..29]
                    type_identifier [25..29]
                navigation_suffix [29..34]
                  . [29..30]
                  simple_identifier [30..34]
              navigation_suffix [34..39]
                . [34..35]
                simple_identifier [35..39]
            call_suffix [39..50]
              type_arguments [39..44]
                type_projection [40..43]
                  user_type [40..43]
                    type_identifier [40..43]
              value_arguments [47..50]
                value_argument [48..49]
                  integer_literal [48..49]
"###,
    );
}

/// `x as a.b.C<T>` at a line's end: lambda across line.
#[test]
fn split_lambda_across_line() {
    same(
        r###"fun f() {
  val a = x as java.util.List<Int>
  { 1 }
}
"###,
        r###"source_file [0..55]
  function_declaration [0..54]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..54]
      statements [12..52]
        property_declaration [12..52]
          binding_pattern_kind [12..15]
            val [12..15]
          variable_declaration [16..17]
            simple_identifier [16..17]
          call_expression [20..52]
            navigation_expression [20..39]
              navigation_expression [20..34]
                as_expression [20..29]
                  simple_identifier [20..21]
                  as [22..24]
                  user_type [25..29]
                    type_identifier [25..29]
                navigation_suffix [29..34]
                  . [29..30]
                  simple_identifier [30..34]
              navigation_suffix [34..39]
                . [34..35]
                simple_identifier [35..39]
            call_suffix [39..52]
              type_arguments [39..44]
                type_projection [40..43]
                  user_type [40..43]
                    type_identifier [40..43]
              annotated_lambda [47..52]
                lambda_literal [47..52]
                  statements [49..50]
                    integer_literal [49..50]
"###,
    );
}

/// `x as a.b.C<T>` at a line's end: for as name.
#[test]
fn split_for_as_name() {
    same(
        r###"fun f() {
  val a = x as java.util.List<Int>
  for (i in l) {}
}
"###,
        r###"source_file [0..65]
  function_declaration [0..64]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..64]
      statements [12..62]
        property_declaration [12..62]
          binding_pattern_kind [12..15]
            val [12..15]
          variable_declaration [16..17]
            simple_identifier [16..17]
          comparison_expression [20..62]
            comparison_expression [20..43]
              navigation_expression [20..39]
                navigation_expression [20..34]
                  as_expression [20..29]
                    simple_identifier [20..21]
                    as [22..24]
                    user_type [25..29]
                      type_identifier [25..29]
                  navigation_suffix [29..34]
                    . [29..30]
                    simple_identifier [30..34]
                navigation_suffix [34..39]
                  . [34..35]
                  simple_identifier [35..39]
              < [39..40]
              simple_identifier [40..43]
            > [43..44]
            call_expression [47..62]
              simple_identifier [47..50]
              call_suffix [51..62]
                value_arguments [51..59]
                  value_argument [52..58]
                    check_expression [52..58]
                      simple_identifier [52..53]
                      in [54..56]
                      simple_identifier [57..58]
                annotated_lambda [60..62]
                  lambda_literal [60..62]
"###,
    );
}

/// `x as a.b.C<T>` at a line's end: while then name keeps type.
#[test]
fn split_while_then_name_keeps_type() {
    same(
        r###"fun f() {
  val a = x as java.util.List<Int>
  while (c) g()
}
"###,
        r###"source_file [0..63]
  function_declaration [0..62]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..62]
      statements [12..60]
        property_declaration [12..44]
          binding_pattern_kind [12..15]
            val [12..15]
          variable_declaration [16..17]
            simple_identifier [16..17]
          as_expression [20..44]
            simple_identifier [20..21]
            as [22..24]
            user_type [25..44]
              type_identifier [25..29]
              type_identifier [30..34]
              type_identifier [35..39]
              type_arguments [39..44]
                type_projection [40..43]
                  user_type [40..43]
                    type_identifier [40..43]
        while_statement [47..60]
          simple_identifier [54..55]
          control_structure_body [57..60]
            call_expression [57..60]
              simple_identifier [57..58]
              call_suffix [58..60]
                value_arguments [58..60]
"###,
    );
}

/// `x as a.b.C<T>` at a line's end: nullable argument keeps type.
#[test]
fn split_nullable_argument_keeps_type() {
    same(
        r###"fun f() {
  val a = x as java.util.List<Int?>
  foo()
}
"###,
        r###"source_file [0..56]
  function_declaration [0..55]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..55]
      statements [12..53]
        property_declaration [12..45]
          binding_pattern_kind [12..15]
            val [12..15]
          variable_declaration [16..17]
            simple_identifier [16..17]
          as_expression [20..45]
            simple_identifier [20..21]
            as [22..24]
            user_type [25..45]
              type_identifier [25..29]
              type_identifier [30..34]
              type_identifier [35..39]
              type_arguments [39..45]
                type_projection [40..44]
                  nullable_type [40..44]
                    user_type [40..43]
                      type_identifier [40..43]
                    quest [43..44]
        call_expression [48..53]
          simple_identifier [48..51]
          call_suffix [51..53]
            value_arguments [51..53]
"###,
    );
}

/// `x as a.b.C<T>` at a line's end: is split.
#[test]
fn split_is_split() {
    same(
        r###"fun f() {
  val a = x is java.util.List<Int>
  assertFailsWith<E> { y }
}
"###,
        r###"source_file [0..74]
  function_declaration [0..73]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..73]
      statements [12..71]
        property_declaration [12..71]
          binding_pattern_kind [12..15]
            val [12..15]
          variable_declaration [16..17]
            simple_identifier [16..17]
          comparison_expression [20..71]
            comparison_expression [20..43]
              navigation_expression [20..39]
                navigation_expression [20..34]
                  check_expression [20..29]
                    simple_identifier [20..21]
                    is [22..24]
                    user_type [25..29]
                      type_identifier [25..29]
                  navigation_suffix [29..34]
                    . [29..30]
                    simple_identifier [30..34]
                navigation_suffix [34..39]
                  . [34..35]
                  simple_identifier [35..39]
              < [39..40]
              simple_identifier [40..43]
            > [43..44]
            call_expression [47..71]
              simple_identifier [47..62]
              call_suffix [62..71]
                type_arguments [62..65]
                  type_projection [63..64]
                    user_type [63..64]
                      type_identifier [63..64]
                annotated_lambda [66..71]
                  lambda_literal [66..71]
                    statements [68..69]
                      simple_identifier [68..69]
"###,
    );
}

/// `x as a.b.C<T>` at a line's end: as question split.
#[test]
fn split_as_question_split() {
    same(
        r###"fun f() {
  val a = x as? java.util.List<Int>
  foo()
}
"###,
        r###"source_file [0..56]
  function_declaration [0..55]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..55]
      statements [12..53]
        property_declaration [12..53]
          binding_pattern_kind [12..15]
            val [12..15]
          variable_declaration [16..17]
            simple_identifier [16..17]
          comparison_expression [20..53]
            comparison_expression [20..44]
              navigation_expression [20..40]
                navigation_expression [20..35]
                  as_expression [20..30]
                    simple_identifier [20..21]
                    as? [22..25]
                    user_type [26..30]
                      type_identifier [26..30]
                  navigation_suffix [30..35]
                    . [30..31]
                    simple_identifier [31..35]
                navigation_suffix [35..40]
                  . [35..36]
                  simple_identifier [36..40]
              < [40..41]
              simple_identifier [41..44]
            > [44..45]
            call_expression [48..53]
              simple_identifier [48..51]
              call_suffix [51..53]
                value_arguments [51..53]
"###,
    );
}

/// `x as a.b.C<T>` at a line's end: split over addition.
#[test]
fn split_split_over_addition() {
    same(
        r###"fun f() {
  val a = y + x as java.util.List<Int>
  foo()
}
"###,
        r###"source_file [0..59]
  function_declaration [0..58]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..58]
      statements [12..56]
        property_declaration [12..56]
          binding_pattern_kind [12..15]
            val [12..15]
          variable_declaration [16..17]
            simple_identifier [16..17]
          comparison_expression [20..56]
            comparison_expression [20..47]
              additive_expression [20..43]
                simple_identifier [20..21]
                + [22..23]
                navigation_expression [24..43]
                  navigation_expression [24..38]
                    as_expression [24..33]
                      simple_identifier [24..25]
                      as [26..28]
                      user_type [29..33]
                        type_identifier [29..33]
                    navigation_suffix [33..38]
                      . [33..34]
                      simple_identifier [34..38]
                  navigation_suffix [38..43]
                    . [38..39]
                    simple_identifier [39..43]
              < [43..44]
              simple_identifier [44..47]
            > [47..48]
            call_expression [51..56]
              simple_identifier [51..54]
              call_suffix [54..56]
                value_arguments [54..56]
"###,
    );
}

/// `x as a.b.C<T>` at a line's end: do while as names.
#[test]
fn split_do_while_as_names() {
    same(
        r###"fun f() {
  val a = x as java.util.List<Int>
  do {} while (c)
}
"###,
        r###"source_file [0..65]
  function_declaration [0..64]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..64]
      statements [12..62]
        property_declaration [12..62]
          binding_pattern_kind [12..15]
            val [12..15]
          variable_declaration [16..17]
            simple_identifier [16..17]
          comparison_expression [20..62]
            comparison_expression [20..43]
              navigation_expression [20..39]
                navigation_expression [20..34]
                  as_expression [20..29]
                    simple_identifier [20..21]
                    as [22..24]
                    user_type [25..29]
                      type_identifier [25..29]
                  navigation_suffix [29..34]
                    . [29..30]
                    simple_identifier [30..34]
                navigation_suffix [34..39]
                  . [34..35]
                  simple_identifier [35..39]
              < [39..40]
              simple_identifier [40..43]
            > [43..44]
            infix_expression [47..62]
              call_expression [47..52]
                simple_identifier [47..49]
                call_suffix [50..52]
                  annotated_lambda [50..52]
                    lambda_literal [50..52]
              simple_identifier [53..58]
              parenthesized_expression [59..62]
                simple_identifier [60..61]
"###,
    );
}
