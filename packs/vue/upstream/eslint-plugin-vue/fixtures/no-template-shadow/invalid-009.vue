
      <template>
        <div v-for="i in 5">
          <div v-for="j in 5">
          </div>
        </div>
      </template>
      <script>
        export default {
          setup() {
            return {
              j: 42
            }
          }
        }
      </script>
      