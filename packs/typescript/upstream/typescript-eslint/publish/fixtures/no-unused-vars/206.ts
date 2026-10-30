
declare const deco: () => ParameterDecorator;

export class Foo {
  bar(@deco() deco, @deco() param) {}
}
      