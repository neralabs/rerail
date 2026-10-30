
interface Htf1 {
  [key: string]: Htf2;
}

interface Htf2 {
  [key: string]: Htf3;
}

interface Htf3 {
  [key: string]: Htf1;
}
    