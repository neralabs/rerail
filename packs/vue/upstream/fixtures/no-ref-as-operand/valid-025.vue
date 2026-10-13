
    <script>
    import { ref, defineComponent } from 'vue'

    export default defineComponent({
      emits: ['incremented'],
      setup(_, { emit }) {
        const counter = ref(0)

        emit('incremented', counter.value, 'xxx')

        return {
          counter
        }
      }
    })
    </script>
    