import React from 'react'
import { useCMS } from '@tinacms/toolkit'

const GetCMS = ({ children }) => {
  let cms
  try {
    cms = useCMS()
  } catch (e) {
    return null
  }
  return <>{children(cms)}</>
}

export default GetCMS
