async function foo() {
	const fs = await import(/* escaped */"\u{66}s/promises");
}