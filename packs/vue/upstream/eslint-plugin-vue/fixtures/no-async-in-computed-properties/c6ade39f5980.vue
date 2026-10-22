
        <script setup>
        import {computed} from 'vue'

        const deepCall = computed(() => z.a.b.c.d().e().f().catch())
        </script>
      