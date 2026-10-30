
type EnthusiasticGreeting<T extends string> = `${Uppercase<T>} - ${Lowercase<T>} - ${Capitalize<T>} - ${Uncapitalize<T>}`;
export type HELLO = EnthusiasticGreeting<"heLLo">;
      