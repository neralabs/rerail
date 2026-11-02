
export class Test {
  constructor(@Optional() value: number[] = []) {
    console.log(value);
  }
}

function Optional() {
  return () => {};
}
    