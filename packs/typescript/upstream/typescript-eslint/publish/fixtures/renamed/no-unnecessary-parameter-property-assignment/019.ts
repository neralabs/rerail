
class Bsw {
  constructor(public bsw: number) {
    this.bsw = bsw;
  }
  srui = (() => {
    this.bsw += 1;
  })();
}
    