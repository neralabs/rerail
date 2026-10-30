
        function ParentComponent() {
          return <Table
            rowRenderer={(rowData) => <Row data={data} />}
          />
        }
      