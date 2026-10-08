
      <script>
        import { ref, computed, toRef, customRef, shallowRef } from 'vue'
        const foo = shallowRef({})
        const isComp = foo.effect
      </script>
    