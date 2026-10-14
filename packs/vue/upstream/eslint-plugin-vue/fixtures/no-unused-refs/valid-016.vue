
      <template>
        <input ref="x" />
      </template>
      <script>
      export default {
        mounted() {
          for(const k in this.$refs) {}
        }
      }
      </script>
      