<script>
function defineComponent(options) { return options }
defineComponent({
  props: ['item']
})
</script>
<template><div v-for="item in items" :key="item"></div></template>
