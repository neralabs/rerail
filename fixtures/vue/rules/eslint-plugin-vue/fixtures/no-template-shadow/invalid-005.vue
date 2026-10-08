<template>
        <div v-for="i in 5"></div>
        <div v-for="f in 5"></div>
      </template>
      <script>
        export default {
          computed: {
            i () { }
          },
          methods: {
            f () { }
          }
        }
      </script>