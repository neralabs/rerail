
const person = {
  firstName: people
    .filter(person => person.firstName.startsWith('s'))
    .map(person => person.firstName)[0],
};
      