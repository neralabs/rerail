
interface Gxp1 {
  [key: string]: Gxp2;
}

interface Gxp2 {
  [key: string]: Gxp3;
}

interface Gxp3 {
  [key: string]: Record<string, Gxp1>;
}
    