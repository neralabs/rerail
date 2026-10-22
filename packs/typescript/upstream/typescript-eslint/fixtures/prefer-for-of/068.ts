
for (let i = 0; i < arr.length; i++) {
  ({ foo: obj[arr[i]] } = { foo: 1 });
}
      