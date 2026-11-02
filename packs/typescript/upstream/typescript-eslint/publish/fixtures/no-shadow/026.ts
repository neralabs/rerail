
function makeA() {
  return class A<T> {
    constructor(public value: T) {}

    static make<T>(value: T) {
      return new A<T>(value);
    }
  };
}
    