<script>
import { defineComponent } from 'elsewhere'
defineComponent({
  props: ['item']
})
</script>
<template><div v-for="item in items" :key="item"></div></template>
