import { useId } from 'vue';

export function useField(labelled) {
  if (labelled) {
    return useId();
  }
  return undefined;
}
