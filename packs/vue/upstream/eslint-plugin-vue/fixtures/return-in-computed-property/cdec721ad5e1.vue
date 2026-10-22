
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { MixedLiterals } from './test01'
        const val = ref<MixedLiterals>(0)
        const result = computed(() => {
          switch (val.value) {
            case 0: return 'zero'
            case 1: return 'one'
            case 'two': return 'two'
            case true: return 'yes'
          }
        })
        </script>