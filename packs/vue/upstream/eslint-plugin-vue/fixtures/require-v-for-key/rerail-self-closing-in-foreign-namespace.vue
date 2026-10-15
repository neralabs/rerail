<template>
  <div>
    <svg>
      <circle v-for="i in 3" />
      <circle v-for="i in 3" :key="i" />
      <g v-for="i in 3"><circle /></g>
    </svg>
    <math>
      <mi v-for="i in 3" />
      <mrow v-for="i in 3"></mrow>
    </math>
    <circle v-for="i in 3" />
    <foo-bar v-for="i in 3" />
    <math>
      <annotation-xml>
        <div v-for="i in 3" />
      </annotation-xml>
      <mi>
        <div v-for="i in 3" />
      </mi>
    </math>
  </div>
</template>
