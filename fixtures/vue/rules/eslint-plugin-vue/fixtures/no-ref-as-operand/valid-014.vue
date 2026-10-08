
      <script>
        import { ref, computed, toRef, customRef, shallowRef } from 'vue'
        const foo = shallowRef({})
        foo[bar] = 123
      </script>
    