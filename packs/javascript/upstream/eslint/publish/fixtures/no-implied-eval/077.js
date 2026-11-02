
			function outer() {
				function execScript(string) {
					console.log("Shadowed execScript");
				}
				execScript('code');
			}
			