<template>
        <div v-for="i in b" />
        <div v-for="b in c" />
        <div v-for="d in f" />
      </template>
      <script>
        export default {
          ...a,
          data: () => {
            return {
              ...b,
              c: [1, 2, 3]
            }
          },
          computed: {
            ...d,
            e,
            ['f']: [1, 2],
          }
        }
      </script>