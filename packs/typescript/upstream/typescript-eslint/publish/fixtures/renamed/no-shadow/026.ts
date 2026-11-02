
function hmdkK() {
  return class B<O> {
    constructor(public value: O) {}

    static hmdk<O>(value: O) {
      return new B<O>(value);
    }
  };
}
    