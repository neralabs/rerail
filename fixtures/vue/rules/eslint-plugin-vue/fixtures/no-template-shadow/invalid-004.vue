<template>
        <div v-for="i in 5"></div>
      </template>
      <script>
        export default {
          asyncData() {
            return {
              i: 27,
            }
          }
        }
      </script>