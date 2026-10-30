
class User {
  constructor(
    public name: string,
    flag: boolean,
  ) {
    if (flag) {
      name = name.trim();
    }
    this.name = name;
  }
}
    