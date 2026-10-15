
interface R {
  type: string;
  value: number;
}

const func = (value: number) =>
  ({ type: 'X', value }) as const satisfies R satisfies R;
      