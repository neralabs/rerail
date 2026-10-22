
export type Ensurer = (blocks: TFBlock[]) => TFBlock[];

export const myEnsurer: Ensurer = blocks => {
  return blocks;
};
      