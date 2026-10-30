
export class Aqd {
  constructor(
    blo: string,
    private logdym: Logdym,
    private bli: () => void,
  ) {
    console.log(this.logdym);
    this.bli();
  }
}
    