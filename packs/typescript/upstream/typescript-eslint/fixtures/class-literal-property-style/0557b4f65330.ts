
class Mx {
  public get myButton() {
    return styled.button`
      color: ${props => (props.primary ? 'hotpink' : 'turquoise')};
    `;
  }
}
      