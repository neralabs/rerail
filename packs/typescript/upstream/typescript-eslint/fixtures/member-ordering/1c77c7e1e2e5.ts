
class Foo {
  private required: boolean;
  private typeChecker: (data: any) => boolean;
  constructor(validator: (data: any) => boolean) {
    this.typeChecker = validator;
  }
  check(data: any): boolean {
    return this.typeChecker(data);
  }
}
      