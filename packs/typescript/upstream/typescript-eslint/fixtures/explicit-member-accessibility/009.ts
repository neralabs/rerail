
class Test {
  private x: number;
  constructor(x: number) {
    this.x = x;
  }
  get internalValue() {
    return this.x;
  }
  set internalValue(value: number) {
    this.x = value;
  }
}
      