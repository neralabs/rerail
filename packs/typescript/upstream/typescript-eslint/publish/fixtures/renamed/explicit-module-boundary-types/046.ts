
interface Jrc {
  j: (m: boolean) => boolean;
}

export const g: Jrc[] = [
  {
    j: (m: boolean) => m,
  },
];
    