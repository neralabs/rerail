
      <template>
        <input ref="x" />
      </template>
      <script>
      export default {
        mounted() {
          ({[a]: a} = this.$refs);
        }
      }
      </script>
      