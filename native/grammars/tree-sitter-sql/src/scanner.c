#include "tree_sitter/parser.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <wctype.h>

enum TokenType {
  DOLLAR_QUOTED_STRING_START_TAG,
  DOLLAR_QUOTED_STRING_END_TAG,
  DOLLAR_QUOTED_STRING,
  COMMENT
};

// Rerail patch 22: a dollar quote's tag is the code points between its two `$`, kept whole (not
// truncated to a byte), at most what one serialization buffer holds after the open flag. A longer
// tag is not read as one.
#define MAX_TAG_LENGTH ((TREE_SITTER_SERIALIZATION_BUFFER_SIZE - 1) / sizeof(int32_t))

typedef struct Tag {
  uint32_t length;
  int32_t chars[MAX_TAG_LENGTH];
} Tag;

typedef struct LexerState {
  // A function body's start tag was read and its end tag not yet.
  bool open;
  Tag start_tag;
} LexerState;

void *tree_sitter_sql_external_scanner_create() {
  return calloc(1, sizeof(LexerState));
}

void tree_sitter_sql_external_scanner_destroy(void *payload) {
  free(payload);
}

// Reads `$tag$` at the lookahead into `tag`: its code points, none of them `$` or white space.
static bool scan_dollar_string_tag(TSLexer *lexer, Tag *tag) {
  if (lexer->lookahead != '$') return false;
  lexer->advance(lexer, false);
  tag->length = 0;
  while (lexer->lookahead != '$' && !iswspace(lexer->lookahead) && !lexer->eof(lexer)) {
    if (tag->length == MAX_TAG_LENGTH) return false;
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

bool tree_sitter_sql_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
  LexerState *state = (LexerState*)payload;
  if (valid_symbols[COMMENT]) {
    while (iswspace(lexer->lookahead)) lexer->advance(lexer, true);
    if (lexer->lookahead == '-' || lexer->lookahead == '/') return scan_comment(lexer);
  }
  if (valid_symbols[DOLLAR_QUOTED_STRING_START_TAG] && !state->open) {
    while (iswspace(lexer->lookahead)) lexer->advance(lexer, true);

    Tag start_tag;
    if (!scan_dollar_string_tag(lexer, &start_tag)) {
      return false;
    }
    state->start_tag = start_tag;
    state->open = true;
    lexer->result_symbol = DOLLAR_QUOTED_STRING_START_TAG;
    return true;
  }

  if (valid_symbols[DOLLAR_QUOTED_STRING_END_TAG] && state->open) {
    while (iswspace(lexer->lookahead)) lexer->advance(lexer, true);

    Tag end_tag;
    if (scan_dollar_string_tag(lexer, &end_tag) && same_tag(&end_tag, &state->start_tag)) {
      state->open = false;
      lexer->result_symbol = DOLLAR_QUOTED_STRING_END_TAG;
      return true;
    }
    return false;
  }

  if (valid_symbols[DOLLAR_QUOTED_STRING]) {
    lexer->mark_end(lexer);
    while (iswspace(lexer->lookahead)) lexer->advance(lexer, true);

    Tag start_tag;
    if (!scan_dollar_string_tag(lexer, &start_tag)) {
      return false;
    }

    if (state->open && same_tag(&state->start_tag, &start_tag)) {
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

// An open start tag is a flag byte and its code points; a closed state is empty.
unsigned tree_sitter_sql_external_scanner_serialize(void *payload, char *buffer) {
  LexerState *state = (LexerState *)payload;
  if (!state->open) {
    return 0;
  }
  unsigned size = state->start_tag.length * sizeof(int32_t);
  buffer[0] = 1;
  memcpy(buffer + 1, state->start_tag.chars, size);
  return 1 + size;
}

void tree_sitter_sql_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {
  LexerState *state = (LexerState *)payload;
  state->open = false;
  state->start_tag.length = 0;
  if (length == 0) {
    return;
  }
  unsigned count = (length - 1) / sizeof(int32_t);
  if (count > MAX_TAG_LENGTH) {
    count = MAX_TAG_LENGTH;
  }
  state->open = true;
  state->start_tag.length = count;
  memcpy(state->start_tag.chars, buffer + 1, count * sizeof(int32_t));
}
