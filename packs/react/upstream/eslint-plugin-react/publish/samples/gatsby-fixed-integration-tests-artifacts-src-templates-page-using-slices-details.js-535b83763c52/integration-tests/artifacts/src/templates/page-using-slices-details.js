import * as React from "react"
import { graphql, Slice } from "gatsby"
import parse from "html-react-parser"

const BlogDetails = ({ data: { sliceBlogPost: post } }) => {
  return (
    <>
      <Slice alias="layout">
        <header>
          <h1 itemProp="headline">{post.title}</h1>
        </header>
        <section itemProp="articleBody">
          {parse(post.content)}
        </section>
        <hr />
        <footer>
          <Slice alias="bio" />
        </footer>
      </Slice>
    </>
  )
}

export default BlogDetails

export const pageQuery = graphql`
  query BlogPostBySlug($id: String!) {
    sliceBlogPost(id: { eq: $id }) {
      id
      title
      authorId
      content
      slug
    }
  }
`
