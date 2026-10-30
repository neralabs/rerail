
export class App {
  constructor(
    baz: string,
    private logger: Logger,
    private bar: () => void,
  ) {
    console.log(this.logger);
    this.bar();
  }
}
    