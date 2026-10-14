#include "tree_sitter/parser.h"

#include <wctype.h>

enum TokenType {
    AUTOMATIC_SEMICOLON,
    TEMPLATE_CHARS,
    TERNARY_QMARK,
    HTML_COMMENT,
    LOGICAL_OR,
    ESCAPE_SEQUENCE,
    REGEX_PATTERN,
    JSX_TEXT,
    FUNCTION_SIGNATURE_AUTOMATIC_SEMICOLON,
    ERROR_RECOVERY,
    // Never returned: valid only after an arrow function's block body.
    ARROW_BODY_END,
    // The `<` of a JSX closing tag whose `/` a comment follows: `<></* c *//>`.
    JSX_CLOSING_TAG_LT,
    // Never returned: valid right after `return` and `yield`, which take no operand after a line break.
    NO_LINE_TERMINATOR_HERE,
};

static void advance(TSLexer *lexer) { lexer->advance(lexer, false); }

static void skip(TSLexer *lexer) { lexer->advance(lexer, true); }

// Whitespace as the grammar's extras read it (`\s`, `\p{Zs}`, U+FEFF, U+2028, U+2029, U+2060, U+200B): every
// ECMAScript WhiteSpace and LineTerminator, such as U+00A0 and U+3000, whatever the C library's locale says.
static bool is_space(int32_t c) {
    switch (c) {
        case '\t':
        case '\n':
        case '\v':
        case '\f':
        case '\r':
        case ' ':
        case 0x0085:
        case 0x00A0:
        case 0x1680:
        case 0x200B:
        case 0x2028:
        case 0x2029:
        case 0x202F:
        case 0x205F:
        case 0x2060:
        case 0x3000:
        case 0xFEFF:
            return true;
        default:
            return c >= 0x2000 && c <= 0x200A;
    }
}

// An ECMAScript LineTerminator: `\n`, `\r`, U+2028 or U+2029 (CRLF reads as one, the `\n` skipped as whitespace).
static bool is_line_terminator(int32_t c) { return c == '\n' || c == '\r' || c == 0x2028 || c == 0x2029; }

// The whitespace JSX text may be laid out with, which is no text before or after a line break: ASCII whitespace only,
// so a U+00A0 or U+3000 stays text whatever the C library's locale says.
static bool is_jsx_layout_space(int32_t c) {
    return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\v' || c == '\f';
}

static bool scan_template_chars(TSLexer *lexer) {
    lexer->result_symbol = TEMPLATE_CHARS;
    for (bool has_content = false;; has_content = true) {
        lexer->mark_end(lexer);
        switch (lexer->lookahead) {
            case '`':
                return has_content;
            case '\0':
                return false;
            case '$':
                advance(lexer);
                if (lexer->lookahead == '{') {
                    return has_content;
                }
                break;
            case '\\':
                return has_content;
            default:
                advance(lexer);
        }
    }
}

static bool scan_whitespace_and_comments(TSLexer *lexer, bool *scanned_comment) {
    for (;;) {
        while (is_space(lexer->lookahead)) {
            skip(lexer);
        }

        if (lexer->lookahead == '/') {
            skip(lexer);

            if (lexer->lookahead == '/') {
                skip(lexer);
                while (lexer->lookahead != 0 && !is_line_terminator(lexer->lookahead)) {
                    skip(lexer);
                }
                *scanned_comment = true;
            } else if (lexer->lookahead == '*') {
                skip(lexer);
                while (lexer->lookahead != 0) {
                    if (lexer->lookahead == '*') {
                        skip(lexer);
                        if (lexer->lookahead == '/') {
                            skip(lexer);
                            break;
                        }
                    } else {
                        skip(lexer);
                    }
                }
            } else {
                return false;
            }
        } else {
            return true;
        }
    }
}

static bool is_identifier_char(int32_t c) { return iswalnum(c) || c == '_' || c == '$'; }

// After `in` or `instanceof` at the start of a line where no binary operator can follow: whether a `:` or `?`
// follows, so the keyword names a member (`in: T`, `instanceof?(): T`) that no expression continues with.
static bool scan_member_name_end(TSLexer *lexer) {
    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
        skip(lexer);
    }
    return lexer->lookahead == ':' || lexer->lookahead == '?';
}

// At a `/` on the statement's line: whether a block comment starts here, ends, and holds a line terminator, which
// ECMAScript reads as one (`a = 1 /*⏎*/ b = 2`, `return /*⏎*/ a`). Consumes the comment if so.
static bool scan_block_comment_line_terminator(TSLexer *lexer) {
    skip(lexer);
    if (lexer->lookahead != '*') {
        return false;
    }
    skip(lexer);
    bool saw_line_terminator = false;
    while (lexer->lookahead != 0) {
        if (lexer->lookahead == '*') {
            skip(lexer);
            if (lexer->lookahead == '/') {
                skip(lexer);
                return saw_line_terminator;
            }
        } else {
            saw_line_terminator |= is_line_terminator(lexer->lookahead);
            skip(lexer);
        }
    }
    return false;
}

static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols, bool *scanned_comment) {
    lexer->result_symbol = AUTOMATIC_SEMICOLON;
    lexer->mark_end(lexer);

    for (;;) {
        if (lexer->lookahead == 0) {
            return true;
        }
        if (lexer->lookahead == '}') {
            // Automatic semicolon insertion breaks detection of object patterns
            // in a typed context:
            //   type F = ({a}: {a: number}) => number;
            // Therefore, disable automatic semicolons when followed by typing
            do {
                skip(lexer);
            } while (is_space(lexer->lookahead));
            if (lexer->lookahead == ':') {
                return valid_symbols[LOGICAL_OR]; // Don't return false if we're in a ternary by checking if || is valid
            }
            return true;
        }
        if (lexer->lookahead == '/') {
            if (!scan_block_comment_line_terminator(lexer)) {
                return false;
            }
            *scanned_comment = true;
            break;
        }
        if (!is_space(lexer->lookahead)) {
            return false;
        }
        if (is_line_terminator(lexer->lookahead)) {
            skip(lexer);
            break;
        }
        skip(lexer);
    }

    // `return` and `yield` take no operand after a line break (ECMAScript's restricted productions), so there the
    // statement ends before the tokens that would otherwise continue it as an operand: `return⏎[a]`, `yield⏎(a)`,
    // `yield⏎/a/`. Before `,`, `;`, `=`, a binary operator or `?` it goes on, as before.
    bool restricted = valid_symbols[NO_LINE_TERMINATOR_HERE];

    if (!scan_whitespace_and_comments(lexer, scanned_comment)) {
        return restricted;
    }

    if (restricted) {
        switch (lexer->lookahead) {
            case '`':
            case '<':
            case '[':
            case '(':
            case '*':
            case '+':
            case '-':
                return true;
            default:
                break;
        }
    }

    // An arrow function is no operand: after its block body, a line that does not go on with a list, a ternary's
    // `:` or a `;` starts a new statement (`f = () => {}⏎[1, 2].map(f)`), where after any other expression it
    // would continue it.
    if (valid_symbols[ARROW_BODY_END]) {
        return lexer->lookahead != ',' && lexer->lookahead != ':' && lexer->lookahead != ';';
    }

    switch (lexer->lookahead) {
        case '`':
        case ',':
        case '.':
        case ';':
        case '%':
        case '>':
        case '=':
        case '?':
        case '^':
        case '|':
        case '&':
        case '/':
        case ':':
            return false;

            // A `<` after a line break never takes type arguments, as TypeScript reads it (its type references take
            // them only on the same line): where no binary operator can come next, as after a type, it starts the
            // next member or statement (`a: Promise<T>⏎<U>(x: U): U`). After an expression it is a comparison or a
            // generic call, and the line goes on.
        case '<':
            return !valid_symbols[LOGICAL_OR];

        case '{':
            if (valid_symbols[FUNCTION_SIGNATURE_AUTOMATIC_SEMICOLON]) {
                return false;
            }
            break;

            // Don't insert a semicolon before a '[', '(' or '*', unless we're parsing
            // a type, or no binary operator can come next, as after a declarator's or a class field's name
            // (`var a⏎[b] = c`, `a⏎*g() {}`). Detect it using the validity of a binary operator token.
        case '(':
        case '[':
        case '*':
            if (valid_symbols[LOGICAL_OR]) {
                return false;
            }
            break;

            // Insert a semicolon before `--` and `++`, and before a `+` or `-` where no binary operator can come
            // next (`var a⏎+b`), but not before binary `+` or `-`.
        case '+':
            skip(lexer);
            return lexer->lookahead == '+' || !valid_symbols[LOGICAL_OR];
        case '-':
            skip(lexer);
            return lexer->lookahead == '-' || !valid_symbols[LOGICAL_OR];

            // Don't insert a semicolon before `!=`, but do insert one before a unary `!`.
        case '!':
            skip(lexer);
            return lexer->lookahead != '=';

            // Don't insert a semicolon before `in` or `instanceof`, but do insert one
            // before an identifier, such as `in_point`, before a member named `in` after a type, or before an
            // `instanceof` that no binary operator can take here: a member's name (`#a⏎instanceof() {}`).
        case 'i':
            skip(lexer);

            if (lexer->lookahead != 'n') {
                return true;
            }
            skip(lexer);

            if (!is_identifier_char(lexer->lookahead)) {
                return !valid_symbols[LOGICAL_OR] && scan_member_name_end(lexer);
            }

            for (unsigned i = 0; i < 8; i++) {
                if (lexer->lookahead != "stanceof"[i]) {
                    return true;
                }
                skip(lexer);
            }

            if (!is_identifier_char(lexer->lookahead)) {
                return !valid_symbols[LOGICAL_OR];
            }
            break;
    }

    return true;
}

static bool scan_ternary_qmark(TSLexer *lexer) {
    for (;;) {
        if (!is_space(lexer->lookahead)) {
            break;
        }
        skip(lexer);
    }

    if (lexer->lookahead == '?') {
        advance(lexer);

        /* Optional chaining. */
        if (lexer->lookahead == '?' || lexer->lookahead == '.') {
            return false;
        }

        lexer->mark_end(lexer);
        lexer->result_symbol = TERNARY_QMARK;

        /* TypeScript optional arguments contain the ?: sequence, possibly
           with whitespace. */
        for (;;) {
            if (!is_space(lexer->lookahead)) {
                break;
            }
            advance(lexer);
        }

        if (lexer->lookahead == ':' || lexer->lookahead == ')' || lexer->lookahead == ',') {
            return false;
        }

        if (lexer->lookahead == '.') {
            advance(lexer);
            if (iswdigit(lexer->lookahead)) {
                return true;
            }
            return false;
        }
        return true;
    }
    return false;
}

// The rest of an HTML comment's line, once its `<!--` or `-->` is read.
static bool scan_html_comment_rest(TSLexer *lexer) {
    while (lexer->lookahead != 0 && !is_line_terminator(lexer->lookahead)) {
        advance(lexer);
    }

    lexer->result_symbol = HTML_COMMENT;
    lexer->mark_end(lexer);

    return true;
}

static bool scan_closing_comment(TSLexer *lexer) {
    while (is_space(lexer->lookahead)) {
        skip(lexer);
    }

    const char *comment_start = "<!--";
    const char *comment_end = "-->";

    if (lexer->lookahead == '<') {
        for (unsigned i = 0; i < 4; i++) {
            if (lexer->lookahead != comment_start[i]) {
                return false;
            }
            advance(lexer);
        }
    } else if (lexer->lookahead == '-') {
        for (unsigned i = 0; i < 3; i++) {
            if (lexer->lookahead != comment_end[i]) {
                return false;
            }
            advance(lexer);
        }
    } else {
        return false;
    }

    return scan_html_comment_rest(lexer);
}

// After an `&`: whether the rest of a character reference follows, as html_character_reference reads one (`&amp;`,
// `&#38;`, `&#x26;`). Consumes the name or number either way.
static bool scan_character_reference_tail(TSLexer *lexer) {
    unsigned length = 0;
    if (lexer->lookahead == '#') {
        advance(lexer);
        if (lexer->lookahead == 'x' || lexer->lookahead == 'X') {
            advance(lexer);
            while (length < 6 && iswxdigit(lexer->lookahead)) {
                advance(lexer);
                length++;
            }
        } else {
            while (length < 5 && iswdigit(lexer->lookahead)) {
                advance(lexer);
                length++;
            }
        }
    } else {
        while (length < 30 && ((lexer->lookahead >= 'a' && lexer->lookahead <= 'z') ||
                               (lexer->lookahead >= 'A' && lexer->lookahead <= 'Z'))) {
            advance(lexer);
            length++;
        }
    }
    return length > 0 && lexer->lookahead == ';';
}

static bool scan_jsx_text(TSLexer *lexer) {
    // saw_text will be true if we see any non-whitespace content, or any whitespace content that is not a newline and
    // does not immediately follow a newline.
    bool saw_text = false;
    // at_newline will be true if we are currently at a newline, or if we are at whitespace that is not a newline but
    // immediately follows a newline.
    bool at_newline = false;

    lexer->result_symbol = JSX_TEXT;

    while (lexer->lookahead != 0 && lexer->lookahead != '<' && lexer->lookahead != '>' && lexer->lookahead != '{' &&
           lexer->lookahead != '}') {
        if (lexer->lookahead == '&') {
            // A character reference is its own token, so the text ends before it. Any other `&` is text, as
            // TypeScript and Babel read it: `Help & Support`, `&nbsp` without its `;`.
            lexer->mark_end(lexer);
            advance(lexer);
            if (scan_character_reference_tail(lexer)) {
                return saw_text;
            }
            saw_text = true;
            at_newline = false;
            continue;
        }
        bool is_wspace = is_jsx_layout_space(lexer->lookahead);
        if (lexer->lookahead == '\n') {
            at_newline = true;
        } else {
            // If at_newline is already true, and we see some whitespace, then it must stay true.
            // Otherwise, it should be false.
            //
            // See the table below to determine the logic for computing `saw_text`.
            //
            // |------------------------------------|
            // | at_newline | is_wspace | saw_text  |
            // |------------|-----------|-----------|
            // | false (0)  | false (0) | true  (1) |
            // | false (0)  | true  (1) | true  (1) |
            // | true  (1)  | false (0) | true  (1) |
            // | true  (1)  | true  (1) | false (0) |
            // |------------------------------------|

            at_newline &= is_wspace;
            if (!at_newline) {
                saw_text = true;
            }
        }

        advance(lexer);
    }

    lexer->mark_end(lexer);
    return saw_text;
}

static inline bool external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
    if (valid_symbols[TEMPLATE_CHARS]) {
        if (valid_symbols[AUTOMATIC_SEMICOLON]) {
            return false;
        }
        return scan_template_chars(lexer);
    }

    if (valid_symbols[JSX_TEXT] && scan_jsx_text(lexer)) {
        return true;
    }

    // A `<` before `/` and a comment, where a JSX closing tag may start: the `<` alone, so the lexer does not read
    // `</` across the comment. Any other `<` is the lexer's, but for an HTML comment's `<!--`, read here as below.
    if (valid_symbols[JSX_CLOSING_TAG_LT] && lexer->lookahead == '<') {
        advance(lexer);
        if (lexer->lookahead == '/') {
            lexer->mark_end(lexer);
            advance(lexer);
            lexer->result_symbol = JSX_CLOSING_TAG_LT;
            return lexer->lookahead == '*';
        }
        if (valid_symbols[HTML_COMMENT] && !valid_symbols[LOGICAL_OR] && !valid_symbols[ESCAPE_SEQUENCE] &&
            !valid_symbols[REGEX_PATTERN]) {
            for (unsigned i = 0; i < 3; i++) {
                if (lexer->lookahead != "!--"[i]) {
                    return false;
                }
                advance(lexer);
            }
            return scan_html_comment_rest(lexer);
        }
        return false;
    }

    if (valid_symbols[AUTOMATIC_SEMICOLON] || valid_symbols[FUNCTION_SIGNATURE_AUTOMATIC_SEMICOLON]) {
        bool scanned_comment = false;
        bool ret = scan_automatic_semicolon(lexer, valid_symbols, &scanned_comment);
        if (!ret && !scanned_comment && valid_symbols[TERNARY_QMARK] && lexer->lookahead == '?') {
            return scan_ternary_qmark(lexer);
        }
        return ret;
    }
    if (valid_symbols[TERNARY_QMARK]) {
        return scan_ternary_qmark(lexer);
    }

    if (valid_symbols[HTML_COMMENT] && !valid_symbols[LOGICAL_OR] && !valid_symbols[ESCAPE_SEQUENCE] &&
        !valid_symbols[REGEX_PATTERN]) {
        return scan_closing_comment(lexer);
    }

    return false;
}
