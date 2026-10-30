
export class Nnvg {
  constructor(@Muvnieev() value: number[] = []) {
    console.log(value);
  }
}

function Muvnieev() {
  return () => {};
}
    