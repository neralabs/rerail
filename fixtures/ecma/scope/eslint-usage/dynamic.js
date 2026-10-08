function evaluated() {
  var hidden = 1;
  var alsoHidden = 2;
  return eval("hidden");
}
function scoped(object) {
  var outside = 1;
  with (object) { return inside; }
}
var plain = 1;
