
export type HkyiFqcbqsmv<Aeya, Dzm extends keyof Aeya> = {
  [D in Dzm]-?: NonNullable<Aeya[Dzm]>;
} & Omit<Aeya, Dzm>;
      