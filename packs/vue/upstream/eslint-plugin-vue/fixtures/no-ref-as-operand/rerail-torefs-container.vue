<script setup>
import { toRefs, ref } from 'vue'
const props = defineProps({ a: Number })
const refs = toRefs(props)
const { a } = toRefs(props)
console.log(refs.a + 1)
console.log(refs.a.value)
console.log(a + 1)
const r = ref(0)
console.log(r + 1)
</script>
