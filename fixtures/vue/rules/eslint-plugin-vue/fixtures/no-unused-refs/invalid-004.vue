
      <template>
        <input ref="x" />
      </template>
      <script>
      export default {
        mounted() {
          this.$refs();
          x[this.$refs];
          this.$refs = foo;
          const {[this.$refs]: a} = foo;
          const [b] = this.$refs;
          ([b] = this.$refs);
        }
      }
      </script>
      