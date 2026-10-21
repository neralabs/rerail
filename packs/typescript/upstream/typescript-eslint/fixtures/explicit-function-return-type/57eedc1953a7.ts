
type HigherOrderType = () => (arg1: string) => (arg2: number) => string;
const x: HigherOrderType = () => arg1 => arg2 => 'foo';
      