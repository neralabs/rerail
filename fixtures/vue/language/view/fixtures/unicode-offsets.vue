<script setup>
const naïve = 'é'
const emoji = '😀'
</script>

<template>
  <p title="ünïcode">😀 {{ naïve }} — {{ emoji + '✓' }}</p>
  <p :data-é="naïve">{{ `${emoji}` }}</p>
</template>
