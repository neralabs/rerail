
      <template>
        <input ref="x" />
      </template>
      <script>
      export default {
        mounted() {
          const {[a]: a} = this.$refs;
        }
      }
      </script>
      