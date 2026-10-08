var one = { b: 1, a: 2 };
var blank = {
  b: 1,

  a: 2,
};
var comment = {
  b: 1,
  // a comment is no blank line
  a: 2,
};
var commented = {
  b: 1,

  // a blank line before the comment
  a: 2,
};
var inside = {
  b: 1, /*

  a blank line inside a comment is not blank

  */ a: 2,
};
var trailing = {
  c: 1, // a trailing comment

  b: 2,
  a: 3,
};
var spaces = {
  b: 1,

  a: 2,
};
var crlf = {
  b: 1,

  a: 2,
};
