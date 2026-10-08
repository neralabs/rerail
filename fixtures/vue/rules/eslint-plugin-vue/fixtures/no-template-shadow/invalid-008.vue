<template>
        <div v-for="i in c" />
        <div v-for="a in c" />
        <div v-for="b in c" />
        <div v-for="d in c" />
        <div v-for="e in f" />
        <div v-for="f in c" />
      </template>
      <script>
        export default {
          ...a,
          data: () => ({
              ...b,
              c: [1, 2, 3]
          }),
          computed: {
            ...d,
            e,
            ['f']: [1, 2],
          }
        }
      </script>