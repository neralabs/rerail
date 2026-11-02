
function a<C, O>(w: () => void): void;
function a<C, O>(w: C extends O ? string : number): void;
      