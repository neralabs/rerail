
var box = { a: 2 };
for (var prop in box) {
  box[prop] = parseInt(box[prop]);
}
    