
interface ITest {
  first: boolean;
  second: string;
  third: boolean;
}

let first = () => console.log('first');

export let second = () => console.log('second');

export namespace Third {
  export let third = () => console.log('third');
}
      