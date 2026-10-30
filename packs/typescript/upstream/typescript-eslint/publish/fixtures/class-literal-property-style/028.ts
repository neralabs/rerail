
class Mx {
  public get myValue() {
    return gql`
      {
        user(id: 5) {
          firstName
          lastName
        }
      }
    `;
  }
}
      