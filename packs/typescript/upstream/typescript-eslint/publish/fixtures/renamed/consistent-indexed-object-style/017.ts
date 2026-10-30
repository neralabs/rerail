
interface Mni<V> {
  [q: string]: V extends true ? Mni : number;
}
    