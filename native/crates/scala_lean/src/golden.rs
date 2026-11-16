//! Golden tests: tree-sitter-scala's tapes of short texts (the engine's `python_tape_expect`,
//! `RERAIL_PY_DIALECT=scala`), against the parser's.

/// The parser's tape of `src`, against tree-sitter's.
fn same(src: &str, want: &str) {
    let got = crate::tape_text(&crate::tape(src.as_bytes()).expect("a tape"));
    assert_eq!(got, want, "{src}");
}

#[test]
fn packages_and_imports() {
    same(
        r###"package a.b

import c.d

class A(x: Int) extends B {
  private val y = 1
  def f(a: Int): Int = a + y
}
"###,
        r###"compilation_unit [0..104]
  package_clause [0..11]
    package_identifier name: [8..11]
      identifier [8..9]
      identifier [10..11]
  import_declaration [13..23]
    identifier path: [20..21]
    . path: [21..22]
    identifier path: [22..23]
  class_definition [25..103]
    class [25..30]
    identifier name: [31..32]
    class_parameters class_parameters: [32..40]
      arguments [32..33]
      class_parameter [33..39]
        identifier name: [33..34]
        type_identifier type: [36..39]
      ) [39..40]
    extends_clause extend: [41..50]
      type_identifier type: [49..50]
    template_body body: [51..103]
      { [51..52]
      val_definition [55..72]
        modifiers [55..62]
          access_modifier [55..62]
            private [55..62]
        identifier pattern: [67..68]
        integer_literal value: {expression} [71..72]
      function_definition [75..101]
        identifier name: [79..80]
        parameters parameters: [80..88]
          arguments [80..81]
          parameter [81..87]
            identifier name: [81..82]
            type_identifier type: [84..87]
          ) [87..88]
        type_identifier return_type: [90..93]
        = [94..95]
        infix_expression body: {expression} [96..101]
          identifier left: [96..97]
          operator_identifier operator: [98..99]
          identifier right: [100..101]
"###,
    );
}

#[test]
fn case_lambdas_and_comments() {
    same(
        r###"object A {
  f { case x =>
    g(x) // c
  }
  h { case y =>
    g(y)
  }
}
"###,
        r###"compilation_unit [0..76]
  object_definition [0..75]
    object [0..6]
    identifier name: [7..8]
    template_body body: [9..75]
      { [9..10]
      call_expression {expression} [13..44]
        identifier function: [13..14]
        case_block arguments: [15..44]
          case_clause [17..35]
            identifier pattern: [22..23]
            => [24..26]
            call_expression body: {expression} [31..35]
              identifier function: [31..32]
              arguments arguments: [32..35]
                arguments [32..33]
                identifier {expression} [33..34]
                ) [34..35]
          comment flags=1 [36..40]
            // [36..38]
      call_expression {expression} [47..73]
        identifier function: [47..48]
        case_block arguments: [49..73]
          case_clause [51..69]
            identifier pattern: [56..57]
            => [58..60]
            call_expression body: {expression} [65..69]
              identifier function: [65..66]
              arguments arguments: [66..69]
                arguments [66..67]
                identifier {expression} [67..68]
                ) [68..69]
"###,
    );
}

#[test]
fn leading_dot_chains() {
    same(
        r###"object A {
  val c = g
    .d(x).w(
      a.S { case y =>
        h(y) // c
      }("m"),
    )
}
"###,
        r###"compilation_unit [0..98]
  object_definition [0..97]
    object [0..6]
    identifier name: [7..8]
    template_body body: [9..97]
      { [9..10]
      val_definition [13..95]
        identifier pattern: [17..18]
        call_expression value: {expression} [21..95]
          field_expression function: [21..34]
            call_expression value: [21..32]
              field_expression function: [21..29]
                identifier value: [21..22]
                identifier field: [28..29]
              arguments arguments: [29..32]
                arguments [29..30]
                identifier {expression} [30..31]
                ) [31..32]
            identifier field: [33..34]
          arguments arguments: [34..95]
            arguments [34..35]
            call_expression {expression} [42..88]
              call_expression function: [42..83]
                field_expression function: [42..45]
                  identifier value: [42..43]
                  identifier field: [44..45]
                case_block arguments: [46..83]
                  case_clause [48..82]
                    identifier pattern: [53..54]
                    => [55..57]
                    call_expression body: {expression} [66..70]
                      identifier function: [66..67]
                      arguments arguments: [67..70]
                        arguments [67..68]
                        identifier {expression} [68..69]
                        ) [69..70]
                    comment flags=1 [71..75]
                      // [71..73]
              arguments arguments: [83..88]
                arguments [83..84]
                string {expression} [84..87]
                ) [87..88]
            , [88..89]
            ) [94..95]
"###,
    );
}

#[test]
fn nested_case_blocks() {
    same(
        r###"object A {
  f(
    a { case y =>
      h(y) // c
    }
  )
}
"###,
        r###"compilation_unit [0..62]
  object_definition [0..61]
    object [0..6]
    identifier name: [7..8]
    template_body body: [9..61]
      { [9..10]
      call_expression {expression} [13..59]
        identifier function: [13..14]
        arguments arguments: [14..59]
          arguments [14..15]
          call_expression {expression} [20..55]
            identifier function: [20..21]
            case_block arguments: [22..55]
              case_clause [24..44]
                identifier pattern: [29..30]
                => [31..33]
                call_expression body: {expression} [40..44]
                  identifier function: [40..41]
                  arguments arguments: [41..44]
                    arguments [41..42]
                    identifier {expression} [42..43]
                    ) [43..44]
              comment flags=1 [45..49]
                // [45..47]
          ) [58..59]
"###,
    );
}

#[test]
fn multiline_extends() {
    same(
        r###"class A(x: Int)
    extends B
    with C {
  def f = 1
}

object D:
  val a = 1

  def g(x: Int): Int =
    x + 1

class E extends F:
  def h = 2
end E
"###,
        r###"compilation_unit [0..152]
  class_definition [0..56]
    class [0..5]
    identifier name: [6..7]
    class_parameters class_parameters: [7..15]
      arguments [7..8]
      class_parameter [8..14]
        identifier name: [8..9]
        type_identifier type: [11..14]
      ) [14..15]
    extends_clause extend: [20..40]
      type_identifier type: [28..29]
      with type: [34..38]
      type_identifier type: [39..40]
    template_body body: [41..56]
      { [41..42]
      function_definition [45..54]
        identifier name: [49..50]
        = [51..52]
        integer_literal body: {expression} [53..54]
  object_definition [58..115]
    object [58..64]
    identifier name: [65..66]
    template_body body: [66..115]
      : [66..67]
      val_definition [70..79]
        identifier pattern: [74..75]
        integer_literal value: {expression} [78..79]
      function_definition [83..115]
        identifier name: [87..88]
        parameters parameters: [88..96]
          arguments [88..89]
          parameter [89..95]
            identifier name: [89..90]
            type_identifier type: [92..95]
          ) [95..96]
        type_identifier return_type: [98..101]
        = [102..103]
        indented_block body: [108..115]
          infix_expression {expression} [108..113]
            identifier left: [108..109]
            operator_identifier operator: [110..111]
            integer_literal right: [112..113]
  class_definition [115..151]
    class [115..120]
    identifier name: [121..122]
    extends_clause extend: [123..132]
      type_identifier type: [131..132]
    template_body body: [132..146]
      : [132..133]
      function_definition [136..145]
        identifier name: [140..141]
        = [142..143]
        integer_literal body: {expression} [144..145]
    end_marker [146..151]
      _end_ident [150..151]
"###,
    );
}

#[test]
fn indented_trait() {
    same(
        r###"trait T:
  def a: Int
  def b = 1
// c
object O
"###,
        r###"compilation_unit [0..48]
  trait_definition [0..39]
    identifier name: [6..7]
    template_body body: [7..39]
      : [7..8]
      function_declaration [11..21]
        identifier name: [15..16]
        type_identifier return_type: [18..21]
      function_definition [24..33]
        identifier name: [28..29]
        = [30..31]
        integer_literal body: {expression} [32..33]
      comment flags=1 [34..38]
        // [34..36]
  object_definition [39..47]
    object [39..45]
    identifier name: [46..47]
"###,
    );
}

#[test]
fn lambdas() {
    same(
        r###"object A {
  val x = () => 1
  f(() => g(1))
  self.f(1)
}
trait B { self: C =>
  def f = 1
}
trait D { this: E with F =>
}
class G { g =>
  val a = 1
}
"###,
        r###"compilation_unit [0..153]
  object_definition [0..58]
    object [0..6]
    identifier name: [7..8]
    template_body body: [9..58]
      { [9..10]
      val_definition [13..28]
        identifier pattern: [17..18]
        lambda_expression value: {expression} [21..28]
          bindings parameters: [21..23]
            arguments [21..22]
            ) [22..23]
          => [24..26]
          integer_literal {expression} [27..28]
      call_expression {expression} [31..44]
        identifier function: [31..32]
        arguments arguments: [32..44]
          arguments [32..33]
          lambda_expression {expression} [33..43]
            bindings parameters: [33..35]
              arguments [33..34]
              ) [34..35]
            => [36..38]
            call_expression {expression} [39..43]
              identifier function: [39..40]
              arguments arguments: [40..43]
                arguments [40..41]
                integer_literal {expression} [41..42]
                ) [42..43]
          ) [43..44]
      call_expression {expression} [47..56]
        field_expression function: [47..53]
          identifier value: [47..51]
          identifier field: [52..53]
        arguments arguments: [53..56]
          arguments [53..54]
          integer_literal {expression} [54..55]
          ) [55..56]
  trait_definition [59..93]
    identifier name: [65..66]
    template_body body: [67..93]
      { [67..68]
      self_type [69..79]
        identifier [69..73]
        type_identifier [75..76]
      function_definition [82..91]
        identifier name: [86..87]
        = [88..89]
        integer_literal body: {expression} [90..91]
  trait_definition [94..123]
    identifier name: [100..101]
    template_body body: [102..123]
      { [102..103]
      self_type [104..121]
        identifier [104..108]
        compound_type [110..118]
          type_identifier base: [110..111]
          type_identifier extra: [117..118]
  class_definition [124..152]
    class [124..129]
    identifier name: [130..131]
    template_body body: [132..152]
      { [132..133]
      self_type [134..138]
        identifier [134..135]
      val_definition [141..150]
        identifier pattern: [145..146]
        integer_literal value: {expression} [149..150]
"###,
    );
}

#[test]
fn annotations_and_modifiers() {
    same(
        r###"@InternalApi
private[akka] class A

package object boo extends C {
  val x = 1
}

object B {
  def f(using c: Ctx): Int = 1
  def g(x: Int)(implicit c: Ctx, d: D): Int = 2
  val m = new M[Int] with N[Int] {
    def h = 3
  }
  /** doc
   */
  final def i[S <: T]: Int = 4
  protected val j: String = "x"
  def k: Int
  val l: Int =
    5
}
"###,
        r###"compilation_unit [0..340]
  class_definition [0..34]
    annotation [0..12]
      type_identifier name: [1..12]
    modifiers [13..26]
      access_modifier [13..26]
        private [13..20]
        access_qualifier [20..26]
          identifier [21..25]
    class [27..32]
    identifier name: [33..34]
  package_object [36..80]
    identifier name: [51..54]
    extends_clause extend: [55..64]
      type_identifier type: [63..64]
    template_body body: [65..80]
      { [65..66]
      val_definition [69..78]
        identifier pattern: [73..74]
        integer_literal value: {expression} [77..78]
  object_definition [82..339]
    object [82..88]
    identifier name: [89..90]
    template_body body: [91..339]
      { [91..92]
      function_definition [95..123]
        identifier name: [99..100]
        parameters parameters: [100..114]
          arguments [100..101]
          using [101..106]
          parameter [107..113]
            identifier name: [107..108]
            type_identifier type: [110..113]
          ) [113..114]
        type_identifier return_type: [116..119]
        = [120..121]
        integer_literal body: {expression} [122..123]
      function_definition [126..171]
        identifier name: [130..131]
        parameters parameters: [131..139]
          arguments [131..132]
          parameter [132..138]
            identifier name: [132..133]
            type_identifier type: [135..138]
          ) [138..139]
        parameters parameters: [139..162]
          arguments [139..140]
          implicit [140..148]
          parameter [149..155]
            identifier name: [149..150]
            type_identifier type: [152..155]
          , [155..156]
          parameter [157..161]
            identifier name: [157..158]
            type_identifier type: [160..161]
          ) [161..162]
        type_identifier return_type: [164..167]
        = [168..169]
        integer_literal body: {expression} [170..171]
      val_definition [174..224]
        identifier pattern: [178..179]
        instance_expression value: {expression} [182..224]
          compound_type [186..204]
            generic_type base: [186..192]
              type_identifier type: [186..187]
              type_arguments type_arguments: [187..192]
                type_identifier [188..191]
            generic_type extra: [198..204]
              type_identifier type: [198..199]
              type_arguments type_arguments: [199..204]
                type_identifier [200..203]
          template_body [205..224]
            { [205..206]
            function_definition [211..220]
              identifier name: [215..216]
              = [217..218]
              integer_literal body: {expression} [219..220]
      block_comment flags=1 [227..240]
      function_definition [243..271]
        modifiers [243..248]
          final [243..248]
        identifier name: [253..254]
        type_parameters parameters: [254..262]
          identifier name: [255..256]
          upper_bound bound: [257..261]
            type_identifier type: [260..261]
        type_identifier return_type: [264..267]
        = [268..269]
        integer_literal body: {expression} [270..271]
      val_definition [274..303]
        modifiers [274..283]
          access_modifier [274..283]
            protected [274..283]
        identifier pattern: [288..289]
        type_identifier type: [291..297]
        string value: {expression} [300..303]
      function_declaration [306..316]
        identifier name: [310..311]
        type_identifier return_type: [313..316]
      val_definition [319..338]
        identifier pattern: [323..324]
        type_identifier type: [326..329]
        indented_block value: [336..338]
          integer_literal {expression} [336..337]
"###,
    );
}

#[test]
fn if_else_and_typed_lambdas() {
    same(
        r###"object A {
  def c(x: Int) = {
    if (x < 1) -1
    else if (x > 1) 1
    else 0
  }
  private sealed trait J
  private object J {
    val a = 1
  }
  f { i: Int =>
    g(i)
  }
  val l =
    m => n(m)
  h(e =
    s => t(s))
}
trait S { self: T =>

  /** doc */
  final def u: Int = 1
}
"###,
        r###"compilation_unit [0..288]
  object_definition [0..227]
    object [0..6]
    identifier name: [7..8]
    template_body body: [9..227]
      { [9..10]
      function_definition [13..85]
        identifier name: [17..18]
        parameters parameters: [18..26]
          arguments [18..19]
          parameter [19..25]
            identifier name: [19..20]
            type_identifier type: [22..25]
          ) [25..26]
        = [27..28]
        block body: {expression} [29..85]
          if_expression {expression} [35..81]
            parenthesized_expression condition: [38..45]
              arguments [38..39]
              infix_expression {expression} [39..44]
                identifier left: [39..40]
                operator_identifier operator: [41..42]
                integer_literal right: [43..44]
              ) [44..45]
            integer_literal consequence: {expression} [46..48]
            else [53..57]
            if_expression alternative: {expression} [58..81]
              parenthesized_expression condition: [61..68]
                arguments [61..62]
                infix_expression {expression} [62..67]
                  identifier left: [62..63]
                  operator_identifier operator: [64..65]
                  integer_literal right: [66..67]
                ) [67..68]
              integer_literal consequence: {expression} [69..70]
              else [75..79]
              integer_literal alternative: {expression} [80..81]
      trait_definition [88..110]
        modifiers [88..102]
          access_modifier [88..95]
            private [88..95]
          sealed [96..102]
        identifier name: [109..110]
      object_definition [113..149]
        modifiers [113..120]
          access_modifier [113..120]
            private [113..120]
        object [121..127]
        identifier name: [128..129]
        template_body body: [130..149]
          { [130..131]
          val_definition [136..145]
            identifier pattern: [140..141]
            integer_literal value: {expression} [144..145]
      call_expression {expression} [152..178]
        identifier function: [152..153]
        block arguments: [154..178]
          lambda_expression [156..177]
            identifier parameters: [156..157]
            : parameters: [157..158]
            type_identifier parameters: [159..162]
            => [163..165]
            indented_block [170..177]
              call_expression {expression} [170..174]
                identifier function: [170..171]
                arguments arguments: [171..174]
                  arguments [171..172]
                  identifier {expression} [172..173]
                  ) [173..174]
      val_definition [181..205]
        identifier pattern: [185..186]
        indented_block value: [193..205]
          lambda_expression {expression} [193..202]
            identifier parameters: [193..194]
            => [195..197]
            call_expression {expression} [198..202]
              identifier function: [198..199]
              arguments arguments: [199..202]
                arguments [199..200]
                identifier {expression} [200..201]
                ) [201..202]
      call_expression {expression} [205..225]
        identifier function: [205..206]
        arguments arguments: [206..225]
          arguments [206..207]
          assignment_expression {expression} [207..224]
            identifier left: [207..208]
            indented_block right: [215..224]
              lambda_expression {expression} [215..224]
                identifier parameters: [215..216]
                => [217..219]
                call_expression {expression} [220..224]
                  identifier function: [220..221]
                  arguments arguments: [221..224]
                    arguments [221..222]
                    identifier {expression} [222..223]
                    ) [223..224]
          ) [224..225]
  trait_definition [228..287]
    identifier name: [234..235]
    template_body body: [236..287]
      { [236..237]
      self_type [238..248]
        identifier [238..242]
        type_identifier [244..245]
      block_comment flags=1 [252..262]
      function_definition [265..285]
        modifiers [265..270]
          final [265..270]
        identifier name: [275..276]
        type_identifier return_type: [278..281]
        = [282..283]
        integer_literal body: {expression} [284..285]
"###,
    );
}

#[test]
fn instance_bodies() {
    same(
        r###"object A {
  val c = new P(x) {
    f(a) = b
  }
  val d = new P(x) {
    g = b
  }
  val e = new P {
    f(a) = b
  }
}
"###,
        r###"compilation_unit [0..121]
  object_definition [0..120]
    object [0..6]
    identifier name: [7..8]
    template_body body: [9..120]
      { [9..10]
      val_definition [13..48]
        identifier pattern: [17..18]
        instance_expression value: {expression} [21..48]
          type_identifier [25..26]
          arguments arguments: [26..29]
            arguments [26..27]
            identifier {expression} [27..28]
            ) [28..29]
          template_body [30..48]
            { [30..31]
            assignment_expression {expression} [36..44]
              call_expression left: [36..40]
                identifier function: [36..37]
                arguments arguments: [37..40]
                  arguments [37..38]
                  identifier {expression} [38..39]
                  ) [39..40]
              identifier right: {expression} [43..44]
      val_definition [51..83]
        identifier pattern: [55..56]
        instance_expression value: {expression} [59..83]
          type_identifier [63..64]
          arguments arguments: [64..67]
            arguments [64..65]
            identifier {expression} [65..66]
            ) [66..67]
          template_body [68..83]
            { [68..69]
            assignment_expression {expression} [74..79]
              identifier left: [74..75]
              identifier right: {expression} [78..79]
      val_definition [86..118]
        identifier pattern: [90..91]
        instance_expression value: {expression} [94..118]
          type_identifier [98..99]
          template_body [100..118]
            { [100..101]
            assignment_expression {expression} [106..114]
              call_expression left: [106..110]
                identifier function: [106..107]
                arguments arguments: [107..110]
                  arguments [107..108]
                  identifier {expression} [108..109]
                  ) [109..110]
              identifier right: {expression} [113..114]
"###,
    );
}

#[test]
fn doc_comments_and_infix_types() {
    same(
        r###"package a

/** x */
@InternalApi
private[akka] case class B(x: Int)

trait C { self: D =>

  /** doc */
  final def e: Int = 1
  def f: Boolean

  /** doc */
  def g: Int = 2
  type ===[A, B] = A Is B
  def h(p: String): AbstractFile | Null = k(p)
}
"###,
        r###"compilation_unit [0..250]
  package_clause [0..9]
    package_identifier name: [8..9]
      identifier [8..9]
  block_comment flags=1 [11..19]
  class_definition [20..67]
    annotation [20..32]
      type_identifier name: [21..32]
    modifiers [33..46]
      access_modifier [33..46]
        private [33..40]
        access_qualifier [40..46]
          identifier [41..45]
    case [47..51]
    class [52..57]
    identifier name: [58..59]
    class_parameters class_parameters: [59..67]
      arguments [59..60]
      class_parameter [60..66]
        identifier name: [60..61]
        type_identifier type: [63..66]
      ) [66..67]
  trait_definition [69..249]
    identifier name: [75..76]
    template_body body: [77..249]
      { [77..78]
      self_type [79..89]
        identifier [79..83]
        type_identifier [85..86]
      block_comment flags=1 [93..103]
      function_definition [106..126]
        modifiers [106..111]
          final [106..111]
        identifier name: [116..117]
        type_identifier return_type: [119..122]
        = [123..124]
        integer_literal body: {expression} [125..126]
      function_declaration [129..143]
        identifier name: [133..134]
        type_identifier return_type: [136..143]
      block_comment flags=1 [147..157]
      function_definition [160..174]
        identifier name: [164..165]
        type_identifier return_type: [167..170]
        = [171..172]
        integer_literal body: {expression} [173..174]
      type_definition [177..200]
        type_identifier name: [182..185]
        type_parameters type_parameters: [185..191]
          identifier name: [186..187]
          identifier name: [189..190]
        = [192..193]
        infix_type type: [194..200]
          type_identifier left: [194..195]
          identifier operator: [196..198]
          type_identifier right: [199..200]
      function_definition [203..247]
        identifier name: [207..208]
        parameters parameters: [208..219]
          arguments [208..209]
          parameter [209..218]
            identifier name: [209..210]
            type_identifier type: [212..218]
          ) [218..219]
        infix_type return_type: [221..240]
          type_identifier left: [221..233]
          operator_identifier operator: [234..235]
          type_identifier right: [236..240]
        = [241..242]
        call_expression body: {expression} [243..247]
          identifier function: [243..244]
          arguments arguments: [244..247]
            arguments [244..245]
            identifier {expression} [245..246]
            ) [246..247]
"###,
    );
}

#[test]
fn given_enum_extension() {
    same(
        r###"import a.{b as c, d}
import e.F as G
import cats.~>

class H(x: Int)(using ec: Executor) {
  def i = 1
}

class J(y: Int)(using Executor, Scheduler):
  def k = 2

object L:
  given M[Int] = N
  given o: P[String] with
    def q = 3
  given R[S] = T(1)

enum Color:
  case Red, Green
  case Mixed(a: Int)

enum Dir(val v: Int):
  case Up extends Dir(1)

trait U:
  extension (s: String)
    def w: Int = 4
  extension (t: Int) def z: Int = 5

object V:
  def a =
    try f()
    catch case e: Throwable => false
"###,
        r###"compilation_unit [0..511]
  import_declaration [0..20]
    identifier path: [7..8]
    . [8..9]
    namespace_selectors [9..20]
      as_renamed_identifier [10..16]
        identifier name: [10..11]
        identifier alias: [15..16]
      identifier [18..19]
  import_declaration [21..36]
    identifier path: [28..29]
    . [29..30]
    as_renamed_identifier [30..36]
      identifier name: [30..31]
      identifier alias: [35..36]
  import_declaration [37..51]
    identifier path: [44..48]
    . path: [48..49]
    operator_identifier path: [49..51]
  class_definition [53..104]
    class [53..58]
    identifier name: [59..60]
    class_parameters class_parameters: [60..68]
      arguments [60..61]
      class_parameter [61..67]
        identifier name: [61..62]
        type_identifier type: [64..67]
      ) [67..68]
    class_parameters class_parameters: [68..88]
      arguments [68..69]
      using [69..74]
      class_parameter [75..87]
        identifier name: [75..77]
        type_identifier type: [79..87]
      ) [87..88]
    template_body body: [89..104]
      { [89..90]
      function_definition [93..102]
        identifier name: [97..98]
        = [99..100]
        integer_literal body: {expression} [101..102]
  class_definition [106..163]
    class [106..111]
    identifier name: [112..113]
    class_parameters class_parameters: [113..121]
      arguments [113..114]
      class_parameter [114..120]
        identifier name: [114..115]
        type_identifier type: [117..120]
      ) [120..121]
    class_parameters class_parameters: [121..148]
      arguments [121..122]
      using [122..127]
      type_identifier type: [128..136]
      , [136..137]
      type_identifier type: [138..147]
      ) [147..148]
    template_body body: [148..163]
      : [148..149]
      function_definition [152..161]
        identifier name: [156..157]
        = [158..159]
        integer_literal body: {expression} [160..161]
  object_definition [163..253]
    object [163..169]
    identifier name: [170..171]
    template_body body: [171..253]
      : [171..172]
      given_definition [175..191]
        given [175..180]
        generic_type return_type: [181..187]
          type_identifier type: [181..182]
          type_arguments type_arguments: [182..187]
            type_identifier [183..186]
        identifier body: {expression} [190..191]
      given_definition [194..234]
        given [194..199]
        identifier name: [200..201]
        : [201..202]
        generic_type return_type: [203..212]
          type_identifier type: [203..204]
          type_arguments type_arguments: [204..212]
            type_identifier [205..211]
        with return_type: [213..217]
        with_template_body body: [222..234]
          function_definition [222..231]
            identifier name: [226..227]
            = [228..229]
            integer_literal body: {expression} [230..231]
      given_definition [234..251]
        given [234..239]
        generic_type return_type: [240..244]
          type_identifier type: [240..241]
          type_arguments type_arguments: [241..244]
            type_identifier [242..243]
        call_expression body: {expression} [247..251]
          identifier function: [247..248]
          arguments arguments: [248..251]
            arguments [248..249]
            integer_literal {expression} [249..250]
            ) [250..251]
  enum_definition [253..305]
    identifier name: [258..263]
    enum_body body: [263..305]
      : [263..264]
      enum_case_definitions [267..282]
        simple_enum_case [272..275]
          identifier name: [272..275]
        simple_enum_case [277..282]
          identifier name: [277..282]
      enum_case_definitions [285..303]
        full_enum_case [290..303]
          identifier name: [290..295]
          class_parameters class_parameters: [295..303]
            arguments [295..296]
            class_parameter [296..302]
              identifier name: [296..297]
              type_identifier type: [299..302]
            ) [302..303]
  enum_definition [305..353]
    identifier name: [310..313]
    class_parameters class_parameters: [313..325]
      arguments [313..314]
      class_parameter [314..324]
        val [314..317]
        identifier name: [318..319]
        type_identifier type: [321..324]
      ) [324..325]
    enum_body body: [325..353]
      : [325..326]
      enum_case_definitions [329..351]
        simple_enum_case [334..351]
          identifier name: [334..336]
          extends_clause extend: [337..351]
            type_identifier type: [345..348]
            arguments arguments: [348..351]
              arguments [348..349]
              integer_literal {expression} [349..350]
              ) [350..351]
  trait_definition [353..442]
    identifier name: [359..360]
    template_body body: [360..442]
      : [360..361]
      extension_definition [364..407]
        extension [364..373]
        parameters parameters: [374..385]
          arguments [374..375]
          parameter [375..384]
            identifier name: [375..376]
            type_identifier type: [378..384]
          ) [384..385]
        function_definition body: [390..404]
          identifier name: [394..395]
          type_identifier return_type: [397..400]
          = [401..402]
          integer_literal body: {expression} [403..404]
      extension_definition [407..440]
        extension [407..416]
        parameters parameters: [417..425]
          arguments [417..418]
          parameter [418..424]
            identifier name: [418..419]
            type_identifier type: [421..424]
          ) [424..425]
        function_definition body: [426..440]
          identifier name: [430..431]
          type_identifier return_type: [433..436]
          = [437..438]
          integer_literal body: {expression} [439..440]
  object_definition [442..511]
    object [442..448]
    identifier name: [449..450]
    template_body body: [450..511]
      : [450..451]
      function_definition [454..511]
        identifier name: [458..459]
        = [460..461]
        indented_block body: [466..511]
          try_expression {expression} [466..510]
            call_expression body: {expression} [470..473]
              identifier function: [470..471]
              arguments arguments: [471..473]
                arguments [471..472]
                ) [472..473]
            catch_clause [478..510]
              typed_pattern pattern: [489..501]
                identifier pattern: [489..490]
                type_identifier type: [492..501]
              boolean_literal body: {expression} [505..510]
                false [505..510]
"###,
    );
}

#[test]
fn self_types_using_kind_projectors() {
    same(
        r###"trait A[V] {
  self: Properties =>

  import RB._
  def b(c: Int)(using Context): Unit = ()
  def d(using q: Quotes)(e: Int) = 1
  val f = new G(h) with I {}
  val g = new J(k, l) with M[N] {
    def o = 1
  }
  def p(): Unit =
    {
      q()
    }
  def r: S[String] =
    new S[String] {
      def t = 1
    }
  def u: T[F[_]] = v[Map[K, *]]
  def w = x(this)
}

object B {
  self =>
  type Erased = Int
}

class C(a: Int) {
  self =>

  def &&(that: C): C = this
  /** doc
    */
  final def thenRun[N <: A]: Int = 1
}
"###,
        r###"compilation_unit [0..523]
  trait_definition [0..364]
    identifier name: [6..7]
    type_parameters type_parameters: [7..10]
      identifier name: [8..9]
    template_body body: [11..364]
      { [11..12]
      self_type [15..34]
        identifier [15..19]
        type_identifier [21..31]
      import_declaration [38..49]
        identifier path: [45..47]
        . [47..48]
        namespace_wildcard [48..49]
          _ [48..49]
      function_definition [52..91]
        identifier name: [56..57]
        parameters parameters: [57..65]
          arguments [57..58]
          parameter [58..64]
            identifier name: [58..59]
            type_identifier type: [61..64]
          ) [64..65]
        parameters parameters: [65..80]
          arguments [65..66]
          using [66..71]
          type_identifier type: [72..79]
          ) [79..80]
        type_identifier return_type: [82..86]
        = [87..88]
        unit body: {expression} [89..91]
          arguments [89..90]
          ) [90..91]
      function_definition [94..128]
        identifier name: [98..99]
        parameters parameters: [99..116]
          arguments [99..100]
          using [100..105]
          parameter [106..115]
            identifier name: [106..107]
            type_identifier type: [109..115]
          ) [115..116]
        parameters parameters: [116..124]
          arguments [116..117]
          parameter [117..123]
            identifier name: [117..118]
            type_identifier type: [120..123]
          ) [123..124]
        = [125..126]
        integer_literal body: {expression} [127..128]
      val_definition [131..157]
        identifier pattern: [135..136]
        instance_expression value: {expression} [139..157]
          compound_type [143..154]
            applied_constructor_type base: [143..147]
              type_identifier [143..144]
              arguments [144..147]
                arguments [144..145]
                identifier {expression} [145..146]
                ) [146..147]
            type_identifier extra: [153..154]
          template_body [155..157]
            { [155..156]
            } [156..157]
      val_definition [160..209]
        identifier pattern: [164..165]
        instance_expression value: {expression} [168..209]
          compound_type [172..189]
            applied_constructor_type base: [172..179]
              type_identifier [172..173]
              arguments [173..179]
                arguments [173..174]
                identifier {expression} [174..175]
                , [175..176]
                identifier {expression} [177..178]
                ) [178..179]
            generic_type extra: [185..189]
              type_identifier type: [185..186]
              type_arguments type_arguments: [186..189]
                type_identifier [187..188]
          template_body [190..209]
            { [190..191]
            function_definition [196..205]
              identifier name: [200..201]
              = [202..203]
              integer_literal body: {expression} [204..205]
      function_definition [212..252]
        identifier name: [216..217]
        parameters parameters: [217..219]
          arguments [217..218]
          ) [218..219]
        type_identifier return_type: [221..225]
        = [226..227]
        indented_block body: [232..252]
          block {expression} [232..249]
            call_expression {expression} [240..243]
              identifier function: [240..241]
              arguments arguments: [241..243]
                arguments [241..242]
                ) [242..243]
      function_definition [252..315]
        identifier name: [256..257]
        generic_type return_type: [259..268]
          type_identifier type: [259..260]
          type_arguments type_arguments: [260..268]
            type_identifier [261..267]
        = [269..270]
        indented_block body: [275..315]
          instance_expression {expression} [275..312]
            generic_type [279..288]
              type_identifier type: [279..280]
              type_arguments type_arguments: [280..288]
                type_identifier [281..287]
            template_body [289..312]
              { [289..290]
              function_definition [297..306]
                identifier name: [301..302]
                = [303..304]
                integer_literal body: {expression} [305..306]
      function_definition [315..344]
        identifier name: [319..320]
        generic_type return_type: [322..329]
          type_identifier type: [322..323]
          type_arguments type_arguments: [323..329]
            generic_type [324..328]
              type_identifier type: [324..325]
              type_arguments type_arguments: [325..328]
                wildcard [326..327]
                  _ [326..327]
        = [330..331]
        generic_function body: {expression} [332..344]
          identifier function: {expression} [332..333]
          type_arguments type_arguments: [333..344]
            generic_type [334..343]
              type_identifier type: [334..337]
              type_arguments type_arguments: [337..343]
                type_identifier [338..339]
                type_identifier [341..342]
      function_definition [347..362]
        identifier name: [351..352]
        = [353..354]
        call_expression body: {expression} [355..362]
          identifier function: [355..356]
          arguments arguments: [356..362]
            arguments [356..357]
            identifier {expression} [357..361]
            ) [361..362]
  object_definition [366..408]
    object [366..372]
    identifier name: [373..374]
    template_body body: [375..408]
      { [375..376]
      self_type [379..386]
        identifier [379..383]
      type_definition [389..406]
        type_identifier name: [394..400]
        = [401..402]
        type_identifier type: [403..406]
  class_definition [410..522]
    class [410..415]
    identifier name: [416..417]
    class_parameters class_parameters: [417..425]
      arguments [417..418]
      class_parameter [418..424]
        identifier name: [418..419]
        type_identifier type: [421..424]
      ) [424..425]
    template_body body: [426..522]
      { [426..427]
      self_type [430..437]
        identifier [430..434]
      function_definition [441..466]
        operator_identifier name: [445..447]
        parameters parameters: [447..456]
          arguments [447..448]
          parameter [448..455]
            identifier name: [448..452]
            type_identifier type: [454..455]
          ) [455..456]
        type_identifier return_type: [458..459]
        = [460..461]
        identifier body: {expression} [462..466]
      block_comment flags=1 [469..483]
      function_definition [486..520]
        modifiers [486..491]
          final [486..491]
        identifier name: [496..503]
        type_parameters parameters: [503..511]
          identifier name: [504..505]
          upper_bound bound: [506..510]
            type_identifier type: [509..510]
        type_identifier return_type: [513..516]
        = [517..518]
        integer_literal body: {expression} [519..520]
"###,
    );
}

#[test]
fn colon_arguments() {
    same(
        r###"object A:
  def a = xs.map: x =>
    x + 1
  test("t"):
    val p = 1
    p
  val w = Writes: d =>
    JsString(d.s)
  def b = c.foreach:
    case 1 => 2
    case _ => 3
  def e = f(1).g: (h, i) =>
    h
  def j = k:
    l
  .m
  def n = o(1):
    p
"###,
        r###"compilation_unit [0..250]
  object_definition [0..250]
    object [0..6]
    identifier name: [7..8]
    template_body body: [8..250]
      : [8..9]
      function_definition [12..45]
        identifier name: [16..17]
        = [18..19]
        call_expression body: {expression} [20..45]
          field_expression function: [20..26]
            identifier value: [20..22]
            identifier field: [23..26]
          colon_argument arguments: [28..45]
            identifier lambda_start: [28..29]
            => lambda_start: [30..32]
            indented_block [37..45]
              infix_expression {expression} [37..42]
                identifier left: [37..38]
                operator_identifier operator: [39..40]
                integer_literal right: [41..42]
      call_expression {expression} [45..78]
        call_expression function: [45..54]
          identifier function: [45..49]
          arguments arguments: [49..54]
            arguments [49..50]
            string {expression} [50..53]
            ) [53..54]
        colon_argument arguments: [60..78]
          indented_block [60..78]
            val_definition [60..69]
              identifier pattern: [64..65]
              integer_literal value: {expression} [68..69]
            identifier {expression} [74..75]
      val_definition [78..119]
        identifier pattern: [82..83]
        call_expression value: {expression} [86..119]
          identifier function: [86..92]
          colon_argument arguments: [94..119]
            identifier lambda_start: [94..95]
            => lambda_start: [96..98]
            indented_block [103..119]
              call_expression {expression} [103..116]
                identifier function: [103..111]
                arguments arguments: [111..116]
                  arguments [111..112]
                  field_expression {expression} [112..115]
                    identifier value: [112..113]
                    identifier field: [114..115]
                  ) [115..116]
      function_definition [119..172]
        identifier name: [123..124]
        = [125..126]
        call_expression body: {expression} [127..172]
          field_expression function: [127..136]
            identifier value: [127..128]
            identifier field: [129..136]
          colon_argument arguments: [142..172]
            indented_cases [142..172]
              case_clause [142..153]
                integer_literal pattern: [147..148]
                => [149..151]
                integer_literal body: {expression} [152..153]
              case_clause [158..169]
                wildcard pattern: [163..164]
                  _ [163..164]
                => [165..167]
                integer_literal body: {expression} [168..169]
      function_definition [172..206]
        identifier name: [176..177]
        = [178..179]
        call_expression body: {expression} [180..206]
          field_expression function: [180..186]
            call_expression value: [180..184]
              identifier function: [180..181]
              arguments arguments: [181..184]
                arguments [181..182]
                integer_literal {expression} [182..183]
                ) [183..184]
            identifier field: [185..186]
          colon_argument arguments: [188..206]
            bindings lambda_start: [188..194]
              arguments [188..189]
              binding [189..190]
                identifier name: [189..190]
              , [190..191]
              binding [192..193]
                identifier name: [192..193]
              ) [193..194]
            => lambda_start: [195..197]
            indented_block [202..206]
              identifier {expression} [202..203]
      function_definition [206..227]
        identifier name: [210..211]
        = [212..213]
        field_expression body: {expression} [214..227]
          call_expression value: [214..225]
            identifier function: [214..215]
            colon_argument arguments: [221..225]
              indented_block [221..225]
                identifier {expression} [221..222]
          identifier field: [226..227]
      function_definition [230..250]
        identifier name: [234..235]
        = [236..237]
        call_expression body: {expression} [238..250]
          call_expression function: [238..242]
            identifier function: [238..239]
            arguments arguments: [239..242]
              arguments [239..240]
              integer_literal {expression} [240..241]
              ) [241..242]
          colon_argument arguments: [248..250]
            indented_block [248..250]
              identifier {expression} [248..249]
"###,
    );
}

#[test]
fn bare_for() {
    same(
        r###"object A:
  def a =
    for
      x <- xs
      y <- ys
    yield x + y
  def b = for x <- xs yield x
  def c =
    for _ <- d
    yield ()
  def e = for x <- xs do f(x)
  def g =
    for
      x <- xs
      if x > 1
    do
      h(x)
"###,
        r###"compilation_unit [0..235]
  object_definition [0..235]
    object [0..6]
    identifier name: [7..8]
    template_body body: [8..235]
      : [8..9]
      function_definition [12..74]
        identifier name: [16..17]
        = [18..19]
        indented_block body: [24..74]
          for_expression {expression} [24..71]
            for [24..27]
            enumerators enumerators: [34..60]
              enumerator [34..41]
                identifier [34..35]
                <- [36..38]
                identifier {expression} [39..41]
              enumerator [48..55]
                identifier [48..49]
                <- [50..52]
                identifier {expression} [53..55]
            yield [60..65]
            infix_expression body: {expression} [66..71]
              identifier left: [66..67]
              operator_identifier operator: [68..69]
              identifier right: [70..71]
      function_definition [74..101]
        identifier name: [78..79]
        = [80..81]
        for_expression body: {expression} [82..101]
          for [82..85]
          enumerators enumerators: [86..93]
            enumerator [86..93]
              identifier [86..87]
              <- [88..90]
              identifier {expression} [91..93]
          yield [94..99]
          identifier body: {expression} [100..101]
      function_definition [104..142]
        identifier name: [108..109]
        = [110..111]
        indented_block body: [116..142]
          for_expression {expression} [116..139]
            for [116..119]
            enumerators enumerators: [120..131]
              enumerator [120..126]
                wildcard [120..121]
                  _ [120..121]
                <- [122..124]
                identifier {expression} [125..126]
            yield [131..136]
            unit body: {expression} [137..139]
              arguments [137..138]
              ) [138..139]
      function_definition [142..169]
        identifier name: [146..147]
        = [148..149]
        for_expression body: {expression} [150..169]
          for [150..153]
          enumerators enumerators: [154..161]
            enumerator [154..161]
              identifier [154..155]
              <- [156..158]
              identifier {expression} [159..161]
          do [162..164]
          call_expression body: {expression} [165..169]
            identifier function: [165..166]
            arguments arguments: [166..169]
              arguments [166..167]
              identifier {expression} [167..168]
              ) [168..169]
      function_definition [172..235]
        identifier name: [176..177]
        = [178..179]
        indented_block body: [184..235]
          for_expression {expression} [184..235]
            for [184..187]
            enumerators enumerators: [194..221]
              enumerator [194..201]
                identifier [194..195]
                <- [196..198]
                identifier {expression} [199..201]
              enumerator [208..216]
                guard [208..216]
                  infix_expression condition: [211..216]
                    identifier left: [211..212]
                    operator_identifier operator: [213..214]
                    integer_literal right: [215..216]
            do [221..223]
            indented_block body: [230..235]
              call_expression {expression} [230..234]
                identifier function: [230..231]
                arguments arguments: [231..234]
                  arguments [231..232]
                  identifier {expression} [232..233]
                  ) [233..234]
"###,
    );
}

#[test]
fn soft_modifiers_postfix_symbols() {
    same(
        r###"package a.b {
  import c.D
  object E
}

package f {
  @Inject
  private object G {
    opaque type H = Int
    private inline def i = 1
    inline given j[T](using K): L[T] = M
    val n = 10 seconds
    val o = p(q, initialDelay = 0 seconds, r = 1)
    val s = t map (u _)
    val v = w.x(y.extension)
    val z = Seq('a, 'b)
    type T <: U
    val a1, b1 = 1
  }
  case class X(a: Int) derives Codec
  @Ann
  case class Y(b: Int)
}
"###,
        r###"compilation_unit [0..436]
  package_clause [0..39]
    package_identifier name: [8..11]
      identifier [8..9]
      identifier [10..11]
    template_body body: [12..39]
      { [12..13]
      import_declaration [16..26]
        identifier path: [23..24]
        . path: [24..25]
        identifier path: [25..26]
      object_definition [29..37]
        object [29..35]
        identifier name: [36..37]
  package_clause [41..435]
    package_identifier name: [49..50]
      identifier [49..50]
    template_body body: [51..435]
      { [51..52]
      object_definition [55..366]
        annotation [55..62]
          type_identifier name: [56..62]
        modifiers [65..72]
          access_modifier [65..72]
            private [65..72]
        object [73..79]
        identifier name: [80..81]
        template_body body: [82..366]
          { [82..83]
          type_definition [88..107]
            opaque_modifier [88..94]
            type_identifier name: [100..101]
            = [102..103]
            type_identifier type: [104..107]
          function_definition [112..136]
            modifiers [112..126]
              access_modifier [112..119]
                private [112..119]
              inline_modifier [120..126]
            identifier name: [131..132]
            = [133..134]
            integer_literal body: {expression} [135..136]
          given_definition [141..177]
            modifiers [141..147]
              inline_modifier [141..147]
            given [148..153]
            identifier name: [154..155]
            type_parameters type_parameters: [155..158]
              identifier name: [156..157]
            parameters parameters: [158..167]
              arguments [158..159]
              using [159..164]
              type_identifier type: [165..166]
              ) [166..167]
            : [167..168]
            generic_type return_type: [169..173]
              type_identifier type: [169..170]
              type_arguments type_arguments: [170..173]
                type_identifier [171..172]
            identifier body: {expression} [176..177]
          val_definition [182..200]
            identifier pattern: [186..187]
            postfix_expression value: {expression} [190..200]
              integer_literal [190..192]
              identifier [193..200]
          val_definition [205..250]
            identifier pattern: [209..210]
            call_expression value: {expression} [213..250]
              identifier function: [213..214]
              arguments arguments: [214..250]
                arguments [214..215]
                identifier {expression} [215..216]
                , [216..217]
                assignment_expression {expression} [218..242]
                  identifier left: [218..230]
                  postfix_expression right: {expression} [233..242]
                    integer_literal [233..234]
                    identifier [235..242]
                , [242..243]
                assignment_expression {expression} [244..249]
                  identifier left: [244..245]
                  integer_literal right: {expression} [248..249]
                ) [249..250]
          val_definition [255..274]
            identifier pattern: [259..260]
            infix_expression value: {expression} [263..274]
              identifier left: [263..264]
              identifier operator: [265..268]
              parenthesized_expression right: [269..274]
                arguments [269..270]
                method_value {expression} [270..273]
                  identifier [270..271]
                  wildcard [272..273]
                    _ [272..273]
                ) [273..274]
          val_definition [279..303]
            identifier pattern: [283..284]
            call_expression value: {expression} [287..303]
              field_expression function: [287..290]
                identifier value: [287..288]
                identifier field: [289..290]
              arguments arguments: [290..303]
                arguments [290..291]
                field_expression {expression} [291..302]
                  identifier value: [291..292]
                  identifier field: [293..302]
                    extension [293..302]
                ) [302..303]
          val_definition [308..327]
            identifier pattern: [312..313]
            call_expression value: {expression} [316..327]
              identifier function: [316..319]
              arguments arguments: [319..327]
                arguments [319..320]
                quote_expression {expression} [320..322]
                  identifier [321..322]
                , [322..323]
                quote_expression {expression} [324..326]
                  identifier [325..326]
                ) [326..327]
          type_definition [332..343]
            type_identifier name: [337..338]
            upper_bound bound: [339..343]
              type_identifier type: [342..343]
          val_definition [348..362]
            identifiers pattern: [352..358]
              identifier [352..354]
              identifier [356..358]
            integer_literal value: {expression} [361..362]
      class_definition [369..403]
        case [369..373]
        class [374..379]
        identifier name: [380..381]
        class_parameters class_parameters: [381..389]
          arguments [381..382]
          class_parameter [382..388]
            identifier name: [382..383]
            type_identifier type: [385..388]
          ) [388..389]
        derives_clause derive: [390..403]
          type_identifier type: [398..403]
      class_definition [406..433]
        annotation [406..410]
          type_identifier name: [407..410]
        case [413..417]
        class [418..423]
        identifier name: [424..425]
        class_parameters class_parameters: [425..433]
          arguments [425..426]
          class_parameter [426..432]
            identifier name: [426..427]
            type_identifier type: [429..432]
          ) [432..433]
"###,
    );
}
