
class Phb {
  constructor(private phb: string) {
    this[`${phb}`] = phb;
  }
}
    