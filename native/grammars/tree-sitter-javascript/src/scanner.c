#include "tree_sitter/parser.h"

#include <stdio.h>
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
    // Never returned: whether the token is valid next (see scan_automatic_semicolon).
    LEFT_BRACKET,
    LEFT_PAREN,
    STAR,
    PLUS,
    MINUS,
    // Never returned: valid only after an arrow function's block body.
    ARROW_BODY_END,
    // The `<` of a JSX closing tag whose `/` a comment follows: `<></* c *//>`.
    JSX_CLOSING_TAG_LT,
    // Never returned: valid right after `return` and `yield`, where ECMAScript allows no line terminator before the
    // operand (see scan_after_line_terminator).
    NO_LINE_TERMINATOR_HERE,
    // Never returned: valid after a class member's `static`, `get` or `set`, where a name on the next line is the
    // member's (see scan_after_line_terminator).
    MEMBER_NAME_HERE,
};

void *tree_sitter_javascript_external_scanner_create() { return NULL; }

void tree_sitter_javascript_external_scanner_destroy(void *p) {}

unsigned tree_sitter_javascript_external_scanner_serialize(void *payload, char *buffer) { return 0; }

void tree_sitter_javascript_external_scanner_deserialize(void *p, const char *b, unsigned n) {}

static inline void advance(TSLexer *lexer) { lexer->advance(lexer, false); }

static inline void skip(TSLexer *lexer) { lexer->advance(lexer, true); }

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

// An ECMAScript LineTerminator: LF, CR, U+2028 and U+2029, as the grammar's `//` comment ends at.
static bool is_line_terminator(int32_t c) { return c == '\n' || c == '\r' || c == 0x2028 || c == 0x2029; }

// Whitespace JSX text may hold between lines, as `iswspace` reads it in the C locale: ASCII alone, so a JSX text's
// range does not depend on the locale (U+00A0 or U+3000 on a line of its own stays text, as React renders it).
static bool is_jsx_space(int32_t c) { return c == ' ' || (c >= '\t' && c <= '\r'); }

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

typedef enum {
    REJECT,     // Semicolon is illegal, ie a syntax error occurred
    NO_NEWLINE, // Unclear if semicolon will be legal, continue
    ACCEPT,     // Semicolon is legal, assuming a comment was encountered
    // A block comment holding a line terminator, which ECMAScript reads as one: semicolon legal as after a line break.
    BLOCK_LINE_TERMINATOR,
} WhitespaceResult;

/**
 * @param consume If false, only consume enough to check if comment indicates semicolon-legality
 */
static WhitespaceResult scan_whitespace_and_comments(TSLexer *lexer, bool *scanned_comment, bool consume) {
    bool saw_block_newline = false;

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
                            *scanned_comment = true;

                            if (lexer->lookahead != '/' && !consume) {
                                return saw_block_newline ? BLOCK_LINE_TERMINATOR : NO_NEWLINE;
                            }

                            break;
                        }
                    } else if (is_line_terminator(lexer->lookahead)) {
                        saw_block_newline = true;
                        skip(lexer);
                    } else {
                        skip(lexer);
                    }
                }
            } else {
                return REJECT;
            }
        } else {
            return ACCEPT;
        }
    }
}

static bool is_identifier_char(int32_t c) { return iswalnum(c) || c == '_' || c == '$'; }

static bool scan_after_line_terminator(TSLexer *lexer, const bool *valid_symbols, bool *scanned_comment);

static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols, bool *scanned_comment) {
    bool comment_condition = !valid_symbols[LOGICAL_OR];
    lexer->result_symbol = AUTOMATIC_SEMICOLON;
    lexer->mark_end(lexer);

    for (;;) {
        if (lexer->lookahead == 0) {
            return true;
        }

        if (lexer->lookahead == '/') {
            WhitespaceResult result = scan_whitespace_and_comments(lexer, scanned_comment, false);
            if (result == REJECT) {
                return false;
            }

            bool block_line_terminator = result == BLOCK_LINE_TERMINATOR;
            if ((result == ACCEPT || block_line_terminator) && comment_condition && lexer->lookahead != ',' &&
                lexer->lookahead != '=') {
                return true;
            }

            // A block comment holding a line terminator is one, as ECMAScript reads it (`a = 1 /*⏎*/ b = 2`): the line
            // ends there. (After a `//` comment, the scanner is called again at the line break it ends at.)
            if (block_line_terminator) {
                return scan_after_line_terminator(lexer, valid_symbols, scanned_comment);
            }
        }

        if (lexer->lookahead == '}') {
            return true;
        }

        if (lexer->is_at_included_range_start(lexer)) {
            return true;
        }

        if (is_line_terminator(lexer->lookahead)) {
            break;
        }

        if (!is_space(lexer->lookahead)) {
            return false;
        }

        skip(lexer);
    }

    skip(lexer);
    return scan_after_line_terminator(lexer, valid_symbols, scanned_comment);
}

// After a line terminator: whether the token starting the next line ends the statement before it.
static bool scan_after_line_terminator(TSLexer *lexer, const bool *valid_symbols, bool *scanned_comment) {
    // After `return` or `yield`, ECMAScript allows no line terminator before the operand: a line break there ends the
    // statement before any token that can start an expression (`return⏎[a]`, `yield⏎(a)`, `return⏎/a/`). A token that
    // cannot (`,`, `;`, `=`, a binary operator) goes on as before.
    bool restricted = valid_symbols[NO_LINE_TERMINATOR_HERE];

    if (scan_whitespace_and_comments(lexer, scanned_comment, true) == REJECT) {
        // A `/` that starts no comment: a regular expression where an operand may start.
        return restricted;
    }

    // An arrow function is no operand: after its block body, a line that does not go on with a list, a ternary's
    // `:` or a `;` starts a new statement (`f = () => {}⏎[1, 2].map(f)`), where after any other expression it
    // would continue it.
    if (valid_symbols[ARROW_BODY_END]) {
        return lexer->lookahead != ',' && lexer->lookahead != ':' && lexer->lookahead != ';';
    }

    // After a class member's `static`, `get` or `set`, a property name on the next line is the member's, as ECMAScript
    // reads a class element (`static get⏎a() {}` is a static getter `a`, `get⏎a() {}` a getter): no semicolon there.
    // `[` and `*` say so through their own symbols; any other token (`=`, `(`, `;`, `}`) goes on as before.
    if (valid_symbols[MEMBER_NAME_HERE] &&
        (is_identifier_char(lexer->lookahead) || lexer->lookahead == '\\' || lexer->lookahead == '#' ||
         lexer->lookahead == '"' || lexer->lookahead == '\'' || lexer->lookahead > 0x7f)) {
        return false;
    }

    switch (lexer->lookahead) {
        case ',':
        case ':':
        case ';':
        case '%':
        case '>':
        case '=':
        case '?':
        case '^':
        case '|':
        case '&':
        case '/':
            return false;

        // A template or a JSX element starts an operand.
        case '`':
        case '<':
            return restricted;

        // Insert a semicolon before a token that cannot continue the statement, as ECMAScript does:
        // `var a⏎[b] = c`, a class field `a⏎*g() {}`. Where the token can, the line continues the statement.
        case '[':
            return restricted || !valid_symbols[LEFT_BRACKET];
        case '(':
            return restricted || !valid_symbols[LEFT_PAREN];
        case '*':
            return restricted || !valid_symbols[STAR];

        // Insert a semicolon before decimals literals but not otherwise.
        case '.':
            skip(lexer);
            return iswdigit(lexer->lookahead);

        // Insert a semicolon before `--` and `++`, and before a `+` or `-` that cannot continue the statement
        // (`var a⏎+b`) or starts a restricted operand (`return⏎-1`), but not before binary `+` or `-`.
        case '+':
            skip(lexer);
            return lexer->lookahead == '+' || restricted || !valid_symbols[PLUS];
        case '-':
            skip(lexer);
            return lexer->lookahead == '-' || restricted || !valid_symbols[MINUS];

        // Don't insert a semicolon before `!=`, but do insert one before a unary `!`.
        case '!':
            skip(lexer);
            return lexer->lookahead != '=';

        // Don't insert a semicolon before `in` or `instanceof`, but do insert one before an identifier, such as
        // `in_point`, or before an `instanceof` that no binary operator can take here: a class member's name.
        case 'i':
            skip(lexer);

            if (lexer->lookahead != 'n') {
                return true;
            }
            skip(lexer);

            if (!is_identifier_char(lexer->lookahead)) {
                return false;
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

        default:
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

        if (lexer->lookahead == '?') {
            return false;
        }

        lexer->mark_end(lexer);
        lexer->result_symbol = TERNARY_QMARK;

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

static bool scan_html_comment(TSLexer *lexer) {
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

// After an `&`: is what follows the rest of an HTML character reference
// (`&amp;`, `&#38;`, `&#x26;`), as the grammar's html_character_reference
// token defines it? Consumes the name or number either way.
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
            // A character reference is its own token: the text ends before it.
            // A bare ampersand (`Shipping & Handling`) is text.
            lexer->mark_end(lexer);
            advance(lexer);
            if (scan_character_reference_tail(lexer)) {
                return saw_text;
            }
            saw_text = true;
            at_newline = false;
            continue;
        }
        bool is_wspace = is_jsx_space(lexer->lookahead);
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

bool tree_sitter_javascript_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
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

    if (valid_symbols[AUTOMATIC_SEMICOLON]) {
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
        return scan_html_comment(lexer);
    }

    return false;
}
