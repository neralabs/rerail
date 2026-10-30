
export {};

export function q(): void;
export function q(v: string): string;
export function q(v?: string): string | void {
  if (v !== undefined) {
    return v;
  }
}
    