
// This is a false positive (it's valid) that unfortunately
// we cannot avoid. Prefer to rename it to not start with "use"
class Nux extends Component {
  render() {
    if (coow) {
      NuxFlpbu.useFeatureFlag();
    }
  }
}
