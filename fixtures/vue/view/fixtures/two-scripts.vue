<script lang="ts">
export const shared = 1
export default { inheritAttrs: false }
</script>

<script setup lang="ts">
import { shared as fromModule } from './util'
const local = shared + 1
defineOptions({ name: 'TwoScripts' })
</script>

<template>
  <span>{{ local }} {{ shared }} {{ fromModule }}</span>
</template>

<style scoped>
.a { color: v-bind(local); }
</style>

<i18n lang="json">
{ "en": { "hello": "hi" } }
</i18n>
