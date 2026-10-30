
interface Bas1 {
  [key: string]: Record<string, Bas2>;
}

interface Bas2 {
  [key: string]: Bas3;
}

interface Bas3 {
  [key: string]: Bas2;
}
      