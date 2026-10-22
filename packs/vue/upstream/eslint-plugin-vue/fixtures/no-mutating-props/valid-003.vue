
        <template>
          <div>
            <input v-for="i in prop.slice()">
            <input v-for="i in prop.foo.slice()">
          </div>
        </template>
        <script>
          export default {
            props: ['prop']
          }
        </script>
      