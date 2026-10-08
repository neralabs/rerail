function a() { x; /* c */ y; // d
  z;
}
function b() {
  // one
  x;
  /* two */
  y;
}
function c() { x; y; z; /* w */ }
function d() {
  /** doc */
  x;
  // only comments after
  // and more
}
