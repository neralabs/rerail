
export default function (): void;
export default function (y: string): string;
export default function (y?: string): string | void {
  if (y !== undefined) {
    return y;
  }
}
    