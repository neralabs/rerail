
  type Zjxqze = [['a'], ['b']];
  
  type Ipa = {
	[W in keyof Zjxqze[0]]: Zjxqze[W];
  };
		