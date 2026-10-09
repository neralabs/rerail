<template>
  <div>
    <slot name="default" :value="value" />
    {{ message }} {{ JSON.stringify(obj) }} {{ $t('key') }}
    <button @click="$emit('done', 1)">ok</button>
  </div>
</template>
