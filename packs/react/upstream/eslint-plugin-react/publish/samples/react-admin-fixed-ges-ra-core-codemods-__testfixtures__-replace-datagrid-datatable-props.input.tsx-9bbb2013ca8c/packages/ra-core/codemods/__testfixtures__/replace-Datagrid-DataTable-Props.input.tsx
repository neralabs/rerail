/* eslint-disable import/no-extraneous-dependencies */
import * as React from 'react';
import { List, Datagrid, TextField, useRecordContext } from 'react-admin';
import parse from 'html-react-parser';

const CustomEmpty = () => <div>No posts found</div>;

const PostPanel = () => {
    const record = useRecordContext();
    return <div>{parse(record.body)}</div>;
};

const postRowStyle = (record, index) => ({
    backgroundColor: record.nb_views >= 500 ? '#efe' : 'white',
});

const PostList = () => (
    <List>
        <Datagrid
            bulkActionButtons={false}
            empty={<CustomEmpty />}
            expand={<PostPanel />}
            expandSingle
            rowClick={false}
            optimized
            rowStyle={postRowStyle}
            sx={{
                '& .RaDatagrid-row': {
                    backgroundColor: 'white',
                },
                '& .RaDatagrid-row:hover': {
                    backgroundColor: '#f5f5f5',
                },
            }}
        >
            <TextField source="id" />
            <TextField source="title" />
            <TextField source="body" />
        </Datagrid>
    </List>
);

export default PostList;
