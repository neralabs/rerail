
export class Test {
  get prop() {
    return 1;
  }
  set prop() {}
  method() {
    return;
  }
  // prettier-ignore
  'method'() {}
  ['prop']() {}
  [`prop`]() {}
  [null]() {}
  [`${v}`](): void {}

  foo = () => {
    bar: 5;
  };
}
      