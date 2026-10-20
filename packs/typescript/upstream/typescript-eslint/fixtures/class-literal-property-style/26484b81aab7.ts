
class Mx {
  public readonly myValue = gql`
    {
      user(id: 5) {
        firstName
        lastName
      }
    }
  `;
}
      