<script>
// Where lizard's Vue reader and Rerail differ on purpose (README, "Deviations"); each is a listed deviation.

// ?? is no decision, as in TypeScript and JavaScript: lizard counts it 2 (5), Rerail 0 (1).
export function withDefaults(options) {
  return options.label ?? options.name ?? "none";
}

// lizard reads a < directly before a letter as the start of a tag and skips to the next >, even lines later, so the
// && and || between them are lost: lizard 1, Rerail 3.
export function outOfRange(low, value, high) {
  return low<value && value<high || value > high;
}
</script>
