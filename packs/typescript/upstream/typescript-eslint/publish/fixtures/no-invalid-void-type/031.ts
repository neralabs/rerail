
export default function (): void;
export default function (x: string): string;
export default function (x?: string): string | void {
  if (x !== undefined) {
    return x;
  }
}
    