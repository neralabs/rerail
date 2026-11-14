#!/usr/bin/env bash
# O125: the 84 files' states over the last 20 commits touching them. 00 = the files before the oldest; NN = the files
# commit NN changed (and .deleted, those it removed).
set -eu
G=$1; R=$2; LIST=$3
rm -rf "$R"; mkdir -p "$R/00"
mapfile -t files < "$LIST"
mapfile -t commits < <(git -C "$G" log --no-merges --format=%H -n 20 -- "${files[@]}" | tail -r)
first=${commits[0]}; n0=0
for f in "${files[@]}"; do
  if git -C "$G" cat-file -e "$first^:$f" 2>/dev/null; then mkdir -p "$R/00/$(dirname "$f")"; git -C "$G" show "$first^:$f" > "$R/00/$f"; n0=$((n0+1)); fi
done
echo "base files $n0"
i=0
for c in "${commits[@]}"; do
  i=$((i+1)); d=$R/$(printf %02d $i); mkdir -p "$d"; : > "$d/.deleted"
  mapfile -t ch < <(git -C "$G" diff-tree --no-commit-id --name-only -r "$c" | grep -Fxf "$LIST" || true)
  for f in "${ch[@]}"; do
    if git -C "$G" cat-file -e "$c:$f" 2>/dev/null; then mkdir -p "$d/$(dirname "$f")"; git -C "$G" show "$c:$f" > "$d/$f"; else echo "$f" >> "$d/.deleted"; fi
  done
  ln=$(git -C "$G" diff --numstat "$c^" "$c" -- "${ch[@]}" | awk '{a+=$1; b+=$2} END{print a"+/"b"-"}')
  echo "$(printf %02d $i) $(git -C "$G" log -1 --format=%h "$c") files ${#ch[@]} deleted $(wc -l < "$d/.deleted" | tr -d ' ') lines $ln"
done
