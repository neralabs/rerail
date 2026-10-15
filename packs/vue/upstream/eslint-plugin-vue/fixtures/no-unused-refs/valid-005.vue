
      <template>
        <input ref="a" />
        <input ref="b" />
        <input ref="c" />
        <input ref="d" />
        <input ref="e" />
      </template>
      <script>
      export default {
        mounted() {
          this.$refs?.a;
          this?.$refs.b;
          $refs?.c;
          this?.$refs?.d;
          (this?.$refs)?.e;
        }
      }
      </script>
      