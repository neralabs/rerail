
type Aft<H extends string = never> = {
  [h in H]: Aft;
};
    