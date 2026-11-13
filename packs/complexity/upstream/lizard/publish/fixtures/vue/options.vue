<script>
// The Options API: methods, computed properties and watchers, each scored as a TypeScript or JavaScript function is.
// lizard numbers a .vue file's lines from its first script block and leaves out the lines between blocks, so every
// file here has one script block, first. The template's expressions are no function's: nothing there is scored.
export default {
  props: { score: Number, late: Boolean, extra: Boolean },
  computed: {
    grade() {
      if (this.score > 90 && !this.late) return "A";
      if (this.score > 80 || this.extra) return "B";
      if (this.score > 70) return "C";
      for (const bonus of [1, 2]) {
        if (bonus > this.score) return "D";
      }
      return "F";
    },
    level() {
      switch (this.score) {
        case 1:
          return "one";
        case 2:
          return "two";
        case 3:
          return "three";
        case 4:
          return "four";
        case 5:
          return "five";
        case 6:
          return "six";
        default:
          return "many";
      }
    },
  },
  watch: {
    score(next, previous) {
      if (next === previous) return;
      if (next > 100 || next < 0) this.$emit("invalid");
      if (next > previous && this.late) this.$emit("late-up");
      if (next < previous) this.$emit("down");
      return next > 50 ? this.$emit("pass") : undefined;
    },
  },
  methods: {
    clamp(value, low, high) {
      if (value < low) return low;
      if (value > high) return high;
      return value;
    },
  },
};
</script>

<template>
  <ul>
    <li v-for="item in items" :key="item.id" :class="item.a ? 'a' : item.b ? 'b' : item.c ? 'c' : 'd'">
      <span v-if="item.a && item.b">{{ item.c || item.d ? 1 : 2 }}</span>
      <span v-else-if="item.e || item.f">{{ item.g && item.h }}</span>
      <button @click="if (item.i) go(); else if (item.j) stop(); else if (item.k) wait();">{{ grade }}</button>
    </li>
  </ul>
</template>
