
      <template>
        <input ref="x" />
        <button @click="alert($refs[foo].value)">Click Me</button>
        <input ref="y" />
        <button @click="alert($refs.z.value)">Click Me</button>
        <input ref="z" />
      </template>
      