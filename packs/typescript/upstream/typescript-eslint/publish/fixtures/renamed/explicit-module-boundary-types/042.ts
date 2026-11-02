
type Ixs = () => (k: number) => string;

export const ixs: Ixs = () => k => String(k);
    