
type Test = {
  // 1
  [Key in // 2
    | 'a' // 3
    | 'b' // 4
    | 'c' // 5
    | 'd' // 6
  ]: string; // 7
  // 8
};
      