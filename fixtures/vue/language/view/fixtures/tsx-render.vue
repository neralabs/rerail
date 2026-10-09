<script lang="tsx">
import { defineComponent, ref } from 'vue'
export default defineComponent({
  setup() {
    const n = ref<number>(1)
    return () => <div onClick={() => n.value++}>{n.value}</div>
  }
})
</script>
