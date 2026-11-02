
export class App {
  constructor(
    baz: string,
    private logger: Logger,
  ) {
    console.log(baz);
    console.log(this.logger);
  }
}
    