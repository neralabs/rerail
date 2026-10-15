
      <template>
        <input ref="x" />
        <input ref="y" />
      </template>
      <script setup>
      import {useTemplateRef} from 'vue'
      const inputX = useTemplateRef('x')
      const inputY = useTemplateRef(`y`)
      </script>
      