<script setup lang="ts">
// <script setup> in TypeScript: functions, arrows assigned to a name, and a computed getter with no name of its own.
import { computed } from "vue";

const props = defineProps<{ score: number; late: boolean; extra: boolean }>();

const grade = computed(() => {
  if (props.score > 90 && !props.late) return "A";
  if (props.score > 80 || props.extra) return "B";
  if (props.score > 70) return "C";
  for (const bonus of [1, 2]) {
    if (bonus > props.score) return "D";
  }
  return "F";
});

function ifElseAtLimit(x: number): string {
  if (x === 1) {
    return "one";
  } else if (x === 2) {
    return "two";
  } else if (x === 3) {
    return "three";
  } else if (x === 4) {
    return "four";
  } else if (x === 5) {
    return "five";
  }
  return "many";
}

const switchOverLimit = (code: string, level: number): string => {
  switch (code) {
    case "a":
      return "alpha";
    case "b":
      return "bravo";
  }
  try {
    if (level > 1) return "high";
    if (level > 2) return "higher";
    if (level > 3) return "highest";
  } catch (error) {
    return "error";
  }
  return level > 4 && level < 9 ? "top" : "low";
};

const simple = (value: number): string => (value === 0 ? "nothing" : String(value));
</script>

<template>
  <p v-if="props.late && props.extra">{{ grade }} {{ ifElseAtLimit(props.score) }} {{ switchOverLimit("a", 1) }}</p>
  <p v-else>{{ simple(props.score) }}</p>
</template>
