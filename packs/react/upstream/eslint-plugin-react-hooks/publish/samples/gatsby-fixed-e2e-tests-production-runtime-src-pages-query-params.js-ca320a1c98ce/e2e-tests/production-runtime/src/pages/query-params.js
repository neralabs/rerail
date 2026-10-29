import React from "react"
import Layout from "../components/layout"

const QueryParams = ({ location }) => {
  const search = location.search
  
  return (
  <Layout>
    <p data-testid="location.search">{search}</p>
  </Layout>
)}

export default QueryParams
