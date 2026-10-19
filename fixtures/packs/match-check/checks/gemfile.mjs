/** Matches a repository with a Gemfile at its root or in any folder. */
export default function gemfile({ files }) {
  const paths = files.filter(path => path === "Gemfile" || path.endsWith("/Gemfile"));
  return paths.length === 0 ? { matched: false } : { matched: true, paths };
}
