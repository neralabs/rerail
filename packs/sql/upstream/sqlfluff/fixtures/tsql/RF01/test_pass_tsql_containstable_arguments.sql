SELECT
    [author_id],
    [rank]
FROM CONTAINSTABLE (
    [dbo].[author_search_documents],
    [search_text],
    @contains_query,
    @candidate_cap
);
