
        <template>
          <div>
            <template v-for="prop of data">
              <input v-model="this.prop">
              <div v-text="prop.shift()"></div>
              <div v-text="this.prop.shift()"></div>
            </template>
          </div>
        </template>
        <script>
          export default {
            props: ['prop']
          }
        </script>
      