
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { BigIntUnion } from './test01'
        const num = ref<BigIntUnion>(1n)
        const result = computed(() => {
          switch (num.value) {
            case 1n: return 'one'
            case 2n: return 'two'
          }
        })
        </script>