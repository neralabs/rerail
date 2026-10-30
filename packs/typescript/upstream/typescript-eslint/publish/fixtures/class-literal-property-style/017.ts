
declare abstract class BaseClass {
  get cursor(): string;
}

class ChildClass extends BaseClass {
  override get cursor() {
    return 'overridden value';
  }
}
      