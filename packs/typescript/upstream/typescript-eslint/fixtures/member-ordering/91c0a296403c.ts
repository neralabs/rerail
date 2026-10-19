
class Foo {
  #bar: string;

  get bar(): string {
    return this.#bar;
  }

  set bar(value: string) {
    this.#bar = value;
  }
}
      