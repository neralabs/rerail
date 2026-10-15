<template>
        <div v-for="i in 5"></div>
        <div v-for="i in f"></div>
        <div v-for="i in 5"></div>
      </template>
      <script>
        export default {
          computed: {
            f () { }
          }
        }
      </script>