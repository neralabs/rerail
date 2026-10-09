
			function outer() {
				function setTimeout(string) {
					console.log("Shadowed setTimeout");
				}
				setTimeout('code');
			}
			