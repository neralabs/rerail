
const command = (): ParameterDecorator => {
  return () => {};
};

export class Foo {
  bar(@command() command: string) {}
}
      