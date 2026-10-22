
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { NumericUnion } from './test01'
        const num = ref<NumericUnion>(1)
        const result = computed(() => {
          switch (num.value) {
            case 1: return 'one'
            case 2: return 'two'
            case 3: return 'three'
          }
        })
        </script>