
a();
try {
  throw new Error();
} catch (foo) {
  var a;
}
      