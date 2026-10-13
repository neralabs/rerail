
			function outer() {
				function setInterval(string) {
					console.log("Shadowed setInterval");
				}
				setInterval('code');
			}
			