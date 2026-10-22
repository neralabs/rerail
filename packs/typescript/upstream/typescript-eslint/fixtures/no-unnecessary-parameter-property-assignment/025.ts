
class User {
  constructor(
    public name: string,
    public age: number,
  ) {
    name = name.trim();
    this.name = name;
  }
}
    