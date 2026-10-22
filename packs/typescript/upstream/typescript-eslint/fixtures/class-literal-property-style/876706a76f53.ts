
declare abstract class BaseClass {
  protected readonly foo: string;
}

class ChildClass extends BaseClass {
  protected override readonly foo = 'bar';
}
      