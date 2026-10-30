
class ScefZvuln {
  [Symbol.iterator](): void;
  [Symbol.iterator](y: string): string;
  [Symbol.iterator](y?: string): string | void {
    if (y !== undefined) {
      return y;
    }
  }
}
    