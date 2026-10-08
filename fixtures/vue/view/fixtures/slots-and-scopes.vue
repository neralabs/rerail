<script setup>
import { ref } from 'vue'
import List from './List.vue'
const selected = ref(null)
const item = 'outer'
</script>

<template>
  <List>
    <template #header="{ title, count = 0 }">
      <h1>{{ title }} ({{ count }})</h1>
    </template>
    <template #item="{ item, index }">
      <li :class="{ active: item === selected }" @click="selected = item">{{ index }}: {{ item }}</li>
    </template>
    <template v-slot:[dynamicSlot]="props">{{ props.text }}</template>
    <template #footer>{{ item }}</template>
  </List>
  <div v-for="n in 3" :key="n">
    <span v-for="m in n" :key="`${n}-${m}`">{{ n * m }}</span>
  </div>
</template>
