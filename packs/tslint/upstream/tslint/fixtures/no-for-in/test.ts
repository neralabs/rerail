const columns = [{ data: [1] }];
for (let i = 0; i < columns[0].data.length; i++) {
	columns.map((x) => x.data[i]);
}

const object = { test: 1, test2: 1, test3: 1 };
for (const key in object) {
	if (object.hasOwnProperty(key)) {
		const element = object[key];
	}
}

for (const key of Object.keys(object)) {
	const value = object[key];
}
