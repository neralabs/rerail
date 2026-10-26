#include "tree_sitter/parser.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <wctype.h>

enum TokenType {
  DOLLAR_QUOTED_STRING,
  COMMENT,
  META_COMMAND,
  COPY_DATA,
  ERROR_SENTINEL,
};

// Rerail patch 22: a dollar quote's tag is the code points between its two `$`, kept whole (not
// truncated to a byte), at most 255 of them. A longer tag is not read as one.
#define MAX_TAG_LENGTH 255

typedef struct Tag {
  uint32_t length;
  int32_t chars[MAX_TAG_LENGTH];
} Tag;

// Rerail patch 24: a dollar-quoted string is one token wherever it occurs, so the scanner keeps no
// state between tokens.
void *tree_sitter_sql_external_scanner_create() {
  return NULL;
}

void tree_sitter_sql_external_scanner_destroy(void *payload) {}

// Rerail patch 23: a tag is empty or an identifier, as PostgreSQL's scanner (scan.l, dolqdelim)
// reads it: a first code point from `[A-Za-z_]` or any non-ASCII one (scan.l takes every byte from
// `\200` up), then those or digits. `$1,$2` is then two parameters, not a tag `1,` whose body runs
// to the next `$1,$`, and a candidate that is no tag fails at its first other code point without
// reading on to look for its close.
static bool is_tag_start(int32_t c) {
  return (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || c == '_' || c >= 0x80;
}

static bool is_tag_char(int32_t c) {
  return is_tag_start(c) || (c >= '0' && c <= '9');
}

// Reads `$tag$` at the lookahead into `tag`: its code points.
static bool scan_dollar_string_tag(TSLexer *lexer, Tag *tag) {
  if (lexer->lookahead != '$') return false;
  lexer->advance(lexer, false);
  tag->length = 0;
  if (lexer->lookahead != '$' && !is_tag_start(lexer->lookahead)) return false;
  while (lexer->lookahead != '$' && !lexer->eof(lexer)) {
    if (tag->length == MAX_TAG_LENGTH || !is_tag_char(lexer->lookahead)) return false;
    tag->chars[tag->length++] = lexer->lookahead;
    lexer->advance(lexer, false);
  }
  if (lexer->lookahead != '$') return false;
  lexer->advance(lexer, false);
  return true;
}

static bool same_tag(const Tag *a, const Tag *b) {
  return a->length == b->length && memcmp(a->chars, b->chars, a->length * sizeof(int32_t)) == 0;
}

// Reads a dollar-quoted string's body and its closing `$tag$`. The delimiter is matched one code
// point at a time; a candidate that fails at a `$` leaves that `$` to start the next candidate, so
// `$a$foo$bar$a$` closes at its last three characters. No tag holds a `$`, so no other restart
// point is missed.
static bool scan_dollar_string_body(TSLexer *lexer, const Tag *tag) {
  while (!lexer->eof(lexer)) {
    if (lexer->lookahead != '$') {
      lexer->advance(lexer, false);
      continue;
    }
    lexer->advance(lexer, false);
    uint32_t matched = 0;
    while (matched < tag->length && !lexer->eof(lexer) && lexer->lookahead == tag->chars[matched]) {
      lexer->advance(lexer, false);
      matched++;
    }
    if (matched == tag->length && !lexer->eof(lexer) && lexer->lookahead == '$') {
      lexer->advance(lexer, false);
      return true;
    }
  }
  return false;
}

// Rerail patch 11: a `--` comment runs to the end of its line; a `/* */` comment nests, as
// PostgreSQL's scanner (scan.l, xc) and the SQL standard read bracketed comments.
static bool scan_comment(TSLexer *lexer) {
  if (lexer->lookahead == '-') {
    lexer->advance(lexer, false);
    if (lexer->lookahead != '-') return false;
    while (!lexer->eof(lexer) && lexer->lookahead != '\n') lexer->advance(lexer, false);
    lexer->mark_end(lexer);
    lexer->result_symbol = COMMENT;
    return true;
  }
  lexer->advance(lexer, false);
  if (lexer->lookahead != '*') return false;
  lexer->advance(lexer, false);
  unsigned depth = 1;
  while (!lexer->eof(lexer)) {
    if (lexer->lookahead == '*') {
      lexer->advance(lexer, false);
      if (lexer->lookahead == '/') {
        lexer->advance(lexer, false);
        if (--depth == 0) {
          lexer->mark_end(lexer);
          lexer->result_symbol = COMMENT;
          return true;
        }
      }
    } else if (lexer->lookahead == '/') {
      lexer->advance(lexer, false);
      if (lexer->lookahead == '*') {
        lexer->advance(lexer, false);
        depth++;
      }
    } else {
      lexer->advance(lexer, false);
    }
  }
  return false;
}

// Rerail patch 25: a psql meta-command (`\echo ...`, `\set ...`, `\gset`), as psql reads a script and
// SQLFluff's postgres dialect lexes it: one `meta_command` token, so nothing inside it is SQL. A
// command is a backslash, its name and its arguments: the rest of the line, up to a backslash that
// starts the next command, or through a `\\` separator. `\set` and `\copy` take the whole line, and
// `\copy (query)` may span lines inside its parentheses. A command that sends the query buffer (`\g`,
// `\gx`, `\gset`, `\gexec`, `\gdesc`, `\crosstabview`, `\watch`) ends the statement before it, as `;`
// does: the scanner leaves it to the grammar's internal _meta_command_send token.
#define MAX_COMMAND_NAME 16

static const char *SEND_COMMANDS[] = {"g", "gx", "gset", "gexec", "gdesc", "crosstabview", "watch"};

static bool is_line_end(TSLexer *lexer) {
  return lexer->eof(lexer) || lexer->lookahead == '\n' || lexer->lookahead == '\r';
}

static void scan_to_line_end(TSLexer *lexer) {
  while (!is_line_end(lexer)) lexer->advance(lexer, false);
}

static bool scan_meta_command(TSLexer *lexer, const bool *valid_symbols) {
  lexer->advance(lexer, false);
  char name[MAX_COMMAND_NAME + 1];
  unsigned length = 0;
  bool long_name = false;
  while (!is_line_end(lexer) && !iswspace(lexer->lookahead) && lexer->lookahead != '\\') {
    if (length < MAX_COMMAND_NAME && lexer->lookahead < 0x80) {
      name[length++] = (char)lexer->lookahead;
    } else {
      long_name = true;
    }
    lexer->advance(lexer, false);
  }
  name[length] = 0;
  if (length == 0 && !long_name) return false;
  bool send = false;
  if (!long_name) {
    for (unsigned i = 0; i < sizeof(SEND_COMMANDS) / sizeof(SEND_COMMANDS[0]); i++) {
      if (strcmp(name, SEND_COMMANDS[i]) == 0) send = true;
    }
  }
  // A command that sends the query buffer is the grammar's internal _meta_command_send token.
  if (send || !valid_symbols[META_COMMAND]) return false;
  lexer->result_symbol = META_COMMAND;
  if (!long_name && (strcmp(name, "set") == 0 || strcmp(name, "copy") == 0)) {
    if (strcmp(name, "copy") == 0) {
      while (!is_line_end(lexer) && iswspace(lexer->lookahead)) lexer->advance(lexer, false);
      if (lexer->lookahead == '(') {
        unsigned depth = 0;
        while (!lexer->eof(lexer)) {
          if (lexer->lookahead == '(') depth++;
          if (lexer->lookahead == ')' && --depth == 0) break;
          lexer->advance(lexer, false);
        }
      }
    }
    scan_to_line_end(lexer);
    lexer->mark_end(lexer);
    return true;
  }
  while (!is_line_end(lexer)) {
    if (lexer->lookahead == '\\') {
      lexer->mark_end(lexer);
      lexer->advance(lexer, false);
      if (lexer->lookahead == '\\') {
        lexer->advance(lexer, false);
        lexer->mark_end(lexer);
      }
      return true;
    }
    lexer->advance(lexer, false);
  }
  lexer->mark_end(lexer);
  return true;
}

// Rerail patch 29: the data rows after `COPY ... FROM STDIN;`. The rest of the `;` line is blank;
// the rows run through the first line that holds `\.` alone (trailing blanks allowed). Without
// such a line there is no data block.
static bool scan_copy_data(TSLexer *lexer) {
  while (lexer->lookahead == ' ' || lexer->lookahead == '\t' || lexer->lookahead == '\r') {
    lexer->advance(lexer, true);
  }
  if (lexer->lookahead != '\n') return false;
  lexer->advance(lexer, true);
  while (!lexer->eof(lexer)) {
    if (lexer->lookahead == '\\') {
      lexer->advance(lexer, false);
      if (lexer->lookahead == '.') {
        lexer->advance(lexer, false);
        while (lexer->lookahead == ' ' || lexer->lookahead == '\t' || lexer->lookahead == '\r') {
          lexer->advance(lexer, false);
        }
        if (lexer->eof(lexer) || lexer->lookahead == '\n') {
          lexer->mark_end(lexer);
          lexer->result_symbol = COPY_DATA;
          return true;
        }
      }
    }
    while (!lexer->eof(lexer) && lexer->lookahead != '\n') lexer->advance(lexer, false);
    if (lexer->eof(lexer)) return false;
    lexer->advance(lexer, false);
  }
  return false;
}

bool tree_sitter_sql_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
  if (valid_symbols[COPY_DATA] && !valid_symbols[ERROR_SENTINEL]) {
    return scan_copy_data(lexer);
  }

  if (valid_symbols[COMMENT] || valid_symbols[META_COMMAND]) {
    while (iswspace(lexer->lookahead)) lexer->advance(lexer, true);
    if (valid_symbols[COMMENT] && (lexer->lookahead == '-' || lexer->lookahead == '/')) return scan_comment(lexer);
    if (lexer->lookahead == '\\') return scan_meta_command(lexer, valid_symbols);
  }

  if (valid_symbols[DOLLAR_QUOTED_STRING]) {
    lexer->mark_end(lexer);
    while (iswspace(lexer->lookahead)) lexer->advance(lexer, true);

    Tag start_tag;
    if (!scan_dollar_string_tag(lexer, &start_tag)) {
      return false;
    }

    if (!scan_dollar_string_body(lexer, &start_tag)) {
      return false;
    }
    lexer->mark_end(lexer);
    lexer->result_symbol = DOLLAR_QUOTED_STRING;
    return true;
  }

  return false;
}

unsigned tree_sitter_sql_external_scanner_serialize(void *payload, char *buffer) {
  return 0;
}

void tree_sitter_sql_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {}
