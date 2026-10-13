
let p1 = 'p1';
class Mx {
  set [/* before set */ p1 /* after set */](val) {}
  get [/* before get */ p1 /* after get */]() {
    return '';
  }
}
    