
interface R {
  type: string;
  value: number;
}

export const func = (value: number) =>
  ({ type: 'X', value }) as const satisfies R satisfies R satisfies R;
      