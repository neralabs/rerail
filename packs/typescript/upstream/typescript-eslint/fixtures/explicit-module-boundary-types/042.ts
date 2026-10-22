
type Buz = () => (n: number) => string;

export const buz: Buz = () => n => String(n);
    