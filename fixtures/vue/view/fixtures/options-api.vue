<script>
export default {
  name: 'OptionsApi',
  props: { value: { type: Number, default: 0 }, label: String },
  data() {
    return { local: 1, rows: [] }
  },
  computed: {
    total() { return this.value + this.local }
  },
  methods: {
    bump(n) { this.local += n }
  }
}
</script>

<template>
  <div :title="label" @click="bump(1)">
    {{ total }} {{ value }} {{ local }} {{ $route.path }}
    <span v-for="(row, i) of rows" :key="i">{{ row.name }} {{ i }} {{ helper(row) }}</span>
  </div>
</template>
