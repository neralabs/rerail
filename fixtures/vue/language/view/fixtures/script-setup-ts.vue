<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import Child from './Child.vue'
const props = defineProps<{ title: string; items: { id: number; label?: string }[] }>()
const emit = defineEmits<{ select: [id: number] }>()
const count = ref(0)
const state = reactive({ open: false })
const doubled = computed(() => count.value * 2)
let mutable = 1
const LIMIT = 10
function select(id: number) { emit('select', id) }
</script>

<template>
  <section :class="{ open: state.open, title }" @click="count++; mutable = 2">
    {{ title.toUpperCase() }} {{ doubled + LIMIT }} {{ Math.max(count, mutable) }} {{ $attrs.id }}
    <Child
      v-for="({ id, label = title }, index) in items"
      :key="id"
      :label="label as string"
      @select="select"
      @close="() => select(id)"
      @open="select(index)"
    />
    <p v-if="count > LIMIT">{{ undeclared }}</p>
    <p v-else-if="props.items.length">many</p>
    <p v-else>none</p>
  </section>
</template>
