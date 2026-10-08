
      <template>
        <input ref="x" />
        <button @click="alert($refs[foo].value)">Click Me</button>
        <input ref="y" />
        <input ref="z" />
      </template>
      