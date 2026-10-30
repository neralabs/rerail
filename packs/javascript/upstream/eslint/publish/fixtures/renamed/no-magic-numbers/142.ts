
  type Vpvih = {
	[0]: 3;
  };
  
  type Glg = {
	[S in keyof Vpvih]: `${S & number}`;
  };
		