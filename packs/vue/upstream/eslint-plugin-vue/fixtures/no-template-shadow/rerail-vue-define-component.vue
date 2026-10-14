<script>
import { defineComponent } from 'vue'
defineComponent({
  props: ['item']
})
</script>
<template><div v-for="item in items" :key="item"></div></template>
