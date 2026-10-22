
      <template>
        <input ref="foo" />
        <input ref="bar" />
      </template>
      <script>
      export default {
        mounted() {
          this.$refs[foo].value = 'foo'
        }
      }
      </script>
      