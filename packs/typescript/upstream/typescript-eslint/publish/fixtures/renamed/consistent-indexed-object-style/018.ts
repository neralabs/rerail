
interface Fid<Y> {
  [m: string]: Y extends true ? string : Fid;
}
    