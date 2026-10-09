<script setup lang="ts">
import { ref } from 'vue'
const text = ref('')
const html = ref('<b>x</b>')
const visible = ref(true)
const attr = ref('data-x')
const vFocus = { mounted: (el: HTMLElement) => el.focus() }
</script>

<template>
  <input v-model.trim="text" v-focus :[attr]="text.length" @keydown.enter.prevent="text = ''" />
  <div v-show="visible" v-html="html" />
  <p v-text="text" v-bind="$attrs" v-on="{ mouseenter: () => (visible = false) }" />
  <p v-memo="[text, visible]" v-once>{{ text }}</p>
  <component :is="visible ? 'span' : 'div'" v-custom:arg.mod="text" />
</template>
