<template>
  <Foo v-for="Foo in list" />
  <div v-for="Foo in list"><Foo /></div>
  <div v-for="Foo in list"><span><Foo /></span></div>
  <Foo v-for="Foo in list"><Foo /></Foo>
  <Foo v-for="Foo in list"><Foo.Bar /></Foo>
  <Foo v-for="Foo in list"></Foo>
  <Foo v-for="Foo in list" :a="Foo"></Foo>
  <Bar #default="{ Foo }"><Foo /></Bar>
  <Bar #default="{ Foo }"></Bar>
  <Bar v-slot="Foo"><Foo /></Bar>
  <Bar v-for="(Foo, Baz) in list"><Baz /></Bar>
  <Bar v-for="(a, Baz) in list"><Baz /></Bar>
  <Bar v-for="(a, b) in list"><Baz :x="b" /></Bar>
  <Bar v-for="{ a = 1 } in list"></Bar>
  <div v-for="a in list" @click="a = 2"></div>
  <div v-for="a in list">{{ a }}</div>
  <div v-for="foo in list"><foo /></div>
</template>
