
        <template>
          <button @click="props.a ++"/>
          <button @click="a ++"/>
          <button @click="props.b.push(1)"/>
          <button @click="b.push(1)"/>
          <input v-model="props.a"/>
          <input v-model="props.a.b"/>
        </template>
        <script setup>
          const props = defineProps({
            a: Number,
            b: Array,
          })
          props.a ++
        </script>
      