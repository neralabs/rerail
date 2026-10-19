
class Test {
  private x: number;
  public constructor(x: number) {
    this.x = x;
  }
  public get internalValue() {
    return this.x;
  }
  public set internalValue(value: number) {
    this.x = value;
  }
  public square(): number {
    return this.x * this.x;
  }
  half(): number {
    return this.x / 2;
  }
}
      